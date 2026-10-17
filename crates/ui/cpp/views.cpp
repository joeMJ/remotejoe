#include "views.h"
#include "editor.h"
#ifdef _WIN32
#include <windows.h>
#else
#include <sys/mman.h>
#include <unistd.h>
#endif

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonDocument>
#include <QtGui/QClipboard>
#include <QtGui/QFontDatabase>
#include <QtGui/QGuiApplication>
#include <QtGui/QKeyEvent>
#include <QtGui/QPainter>
#include <QtGui/QWheelEvent>
#include <QtWidgets/QApplication>
#include <QtCore/QPointer>
#include <QtCore/QTimer>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QMenu>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QStyle>
#include <QtWidgets/QVBoxLayout>

// ---------- FrameView ----------

FrameView::FrameView(RjBackend* backend, std::uint64_t session, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
  , m_session(session)
{
	setFocusPolicy(Qt::StrongFocus);
	setMouseTracking(true);
	setCursor(Qt::ArrowCursor);
	setAttribute(Qt::WA_OpaquePaintEvent);
	setMinimumSize(160, 120);
	// Größenänderungen des Fensters erst nach einer kurzen Pause weitergeben,
	// damit beim Ziehen nicht bei jedem Pixel neu aufgebaut wird.
	m_fitTimer = new QTimer(this);
	m_fitTimer->setSingleShot(true);
	m_fitTimer->setInterval(400);
	connect(m_fitTimer, &QTimer::timeout, this, [this] { requestFit(); });
}

FrameView::~FrameView()
{
	unmap();
}

void FrameView::unmap()
{
	m_image = QImage();
	if (!m_map)
		return;
#ifdef _WIN32
	UnmapViewOfFile(m_map);
#else
	munmap(m_map, m_mapLen);
#endif
	m_map = nullptr;
	m_mapLen = 0;
}

void FrameView::closeHandle(std::int64_t fd)
{
	if (fd < 0)
		return;
#ifdef _WIN32
	CloseHandle(reinterpret_cast<HANDLE>(static_cast<std::intptr_t>(fd)));
#else
	::close(int(fd));
#endif
}

void FrameView::attach(std::int64_t fd, int w, int h)
{
	// Der Hauptprozess hat Größe und Versiegelung bereits geprüft; die
	// Kennung wird nach dem Abbilden geschlossen.
	unmap();
	const std::size_t len = std::size_t(w) * std::size_t(h) * 4;
	if (fd >= 0 && len > 0)
	{
#ifdef _WIN32
		void* p = MapViewOfFile(reinterpret_cast<HANDLE>(static_cast<std::intptr_t>(fd)), FILE_MAP_READ, 0, 0, len);
		if (p)
		{
			m_map = static_cast<uchar*>(p);
			m_mapLen = len;
		}
#else
		void* p = mmap(nullptr, len, PROT_READ, MAP_SHARED, int(fd), 0);
		if (p != MAP_FAILED)
		{
			m_map = static_cast<uchar*>(p);
			m_mapLen = len;
		}
#endif
	}
	closeHandle(fd);
	if (m_map)
		// BGRA im Speicher entspricht QImage::Format_RGB32 (little endian).
		m_image = QImage(m_map, w, h, w * 4, QImage::Format_RGB32);
	// Neue Verbindung oder von der Gegenstelle bestimmte Größe (nicht die Antwort auf
	// unsere Anfrage): einmal an das Fenster anpassen, höchstens dreimal hintereinander.
	const QSize now(w, h);
	const bool ours = m_requested.isValid() && qAbs(now.width() - m_requested.width()) < 8 && qAbs(now.height() - m_requested.height()) < 8;
	if (m_map && !ours && now != m_attached)
	{
		m_requested = QSize();
		if (m_autoFits < 3)
		{
			++m_autoFits;
			m_fitTimer->start();
		}
	}
	m_attached = now;
	update();
}

void FrameView::resizeEvent(QResizeEvent*)
{
	m_autoFits = 0;
	m_fitTimer->start();
}

void FrameView::requestFit()
{
	if (m_image.isNull() || !isVisible() || width() < 200 || height() < 150)
		return;
	const int w = qBound(200, width() & ~1, 8192);
	const int h = qBound(200, height(), 8192);
	if (qAbs(w - m_image.width()) < 8 && qAbs(h - m_image.height()) < 8)
		return;
	if (QSize(w, h) == m_requested)
		return;
	m_requested = QSize(w, h);
	m_backend->resizeSession(m_session, std::uint32_t(w), std::uint32_t(h));
}

QRect FrameView::target() const
{
	if (m_image.isNull())
		return rect();
	QSize s = m_image.size().scaled(size(), Qt::KeepAspectRatio);
	if (s.width() > m_image.width())
		s = m_image.size();
	return QRect(QPoint((width() - s.width()) / 2, (height() - s.height()) / 2), s);
}

void FrameView::damage(int, int, int, int)
{
	update();
}

void FrameView::paintEvent(QPaintEvent*)
{
	QPainter p(this);
	p.fillRect(rect(), palette().window());
	if (m_image.isNull())
	{
		p.setPen(palette().text().color());
		p.drawText(rect(), Qt::AlignCenter, m_backend->text(QStringLiteral("state.connecting")));
		return;
	}
	p.setRenderHint(QPainter::SmoothPixmapTransform);
	p.drawImage(target(), m_image);
	if (hasFocus() && s_focusFrame)
	{
		p.setPen(QPen(palette().highlight().color(), 2));
		p.drawRect(rect().adjusted(1, 1, -1, -1));
	}
}

QPoint FrameView::toRemote(const QPointF& pos) const
{
	QRect t = target();
	if (m_image.isNull() || t.width() == 0 || t.height() == 0)
		return QPoint();
	int x = int((pos.x() - t.x()) * m_image.width() / t.width());
	int y = int((pos.y() - t.y()) * m_image.height() / t.height());
	return QPoint(qBound(0, x, m_image.width() - 1), qBound(0, y, m_image.height() - 1));
}

static int buttonId(Qt::MouseButton b)
{
	switch (b)
	{
		case Qt::LeftButton:
			return 1;
		case Qt::RightButton:
			return 2;
		case Qt::MiddleButton:
			return 3;
		default:
			return 0;
	}
}

void FrameView::mouseMoveEvent(QMouseEvent* e)
{
	QPoint r = toRemote(e->position());
	m_backend->mouseEvent(m_session, r.x(), r.y(), 0, false);
}

void FrameView::mousePressEvent(QMouseEvent* e)
{
	setFocus();
	QPoint r = toRemote(e->position());
	m_backend->mouseEvent(m_session, r.x(), r.y(), buttonId(e->button()), true);
}

void FrameView::mouseReleaseEvent(QMouseEvent* e)
{
	QPoint r = toRemote(e->position());
	m_backend->mouseEvent(m_session, r.x(), r.y(), buttonId(e->button()), false);
}

void FrameView::wheelEvent(QWheelEvent* e)
{
	QPoint r = toRemote(e->position());
	m_backend->mouseEvent(m_session, r.x(), r.y(), e->angleDelta().y() > 0 ? 4 : 5, true);
}

void FrameView::keyPressEvent(QKeyEvent* e)
{
	m_backend->keyEvent(m_session, std::uint32_t(e->key()), std::uint32_t(e->modifiers()), e->text(), true);
}

void FrameView::keyReleaseEvent(QKeyEvent* e)
{
	if (!e->isAutoRepeat())
		m_backend->keyEvent(m_session, std::uint32_t(e->key()), std::uint32_t(e->modifiers()), e->text(), false);
}

void FrameView::focusInEvent(QFocusEvent* e)
{
	QWidget::focusInEvent(e);
	// Lokale Zwischenablage beim Betreten anbieten (Richtung prüft der Worker).
	const QString text = QGuiApplication::clipboard()->text();
	if (!text.isEmpty())
		m_backend->offerClipboard(m_session, text);
	if (m_web)
		m_backend->edit(QStringLiteral("{\"op\":\"web_command\",\"session\":%1,\"cmd\":{\"op\":\"focus\",\"on\":true}}").arg(m_session));
	update();
}

void FrameView::focusOutEvent(QFocusEvent* e)
{
	QWidget::focusOutEvent(e);
	if (m_web)
		m_backend->edit(QStringLiteral("{\"op\":\"web_command\",\"session\":%1,\"cmd\":{\"op\":\"focus\",\"on\":false}}").arg(m_session));
	update();
}

QPoint FrameView::toLocal(const QPoint& remote) const
{
	const QRect t = target();
	if (m_image.isNull() || m_image.width() == 0 || m_image.height() == 0)
		return remote;
	return QPoint(t.x() + remote.x() * t.width() / m_image.width(), t.y() + remote.y() * t.height() / m_image.height());
}

// ---------- TerminalView ----------

TerminalView::TerminalView(RjBackend* backend, std::uint64_t session, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
  , m_session(session)
{
	setFocusPolicy(Qt::StrongFocus);
	setAttribute(Qt::WA_InputMethodEnabled);
	setCursor(Qt::IBeamCursor);
	m_font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
	m_font.setPointSize(11);
	QFontMetrics fm(m_font);
	m_cw = qMax(1, fm.horizontalAdvance(QLatin1Char('M')));
	m_ch = qMax(1, fm.height());
	m_ascent = fm.ascent();
	setMinimumSize(m_cw * 20, m_ch * 5);
	applySettings(QJsonDocument::fromJson(m_backend->terminalSettings().toUtf8()).object());
}

namespace
{
bool s_copyOnSelect = true;
bool s_confirmPaste = true;
QPointer<TerminalView> s_activeTerminal;
} // namespace

TerminalView* TerminalView::active()
{
	return s_activeTerminal && s_activeTerminal->isVisible() ? s_activeTerminal.data() : nullptr;
}

void TerminalView::focusInEvent(QFocusEvent* e)
{
	QWidget::focusInEvent(e);
	s_activeTerminal = this;
	update();
}

void TerminalView::applySettings(const QJsonObject& s)
{
	if (s.isEmpty())
		return;
	s_copyOnSelect = s.value(QStringLiteral("copy_on_select")).toBool(true);
	s_confirmPaste = s.value(QStringLiteral("confirm_paste")).toBool(true);
	const QString family = s.value(QStringLiteral("font_family")).toString();
	QFont f = family.isEmpty() ? QFontDatabase::systemFont(QFontDatabase::FixedFont) : QFont(family);
	f.setStyleHint(QFont::Monospace);
	f.setFixedPitch(true);
	f.setPointSizeF(qBound(6.0, s.value(QStringLiteral("font_size")).toDouble(11.0), 40.0));
	m_font = f;
	const QFontMetrics fm(m_font);
	m_cw = qMax(1, fm.horizontalAdvance(QLatin1Char('M')));
	m_ch = qMax(1, fm.height());
	m_ascent = fm.ascent();
	const QColor sel(s.value(QStringLiteral("selection")).toString());
	m_selection = sel.isValid() ? sel : QColor();
	setMinimumSize(m_cw * 20, m_ch * 5);
	// Zellengröße hat sich geändert: Gitter der Sitzung neu bemessen.
	const QSize c = cells();
	if (c != m_lastSize)
	{
		m_lastSize = c;
		m_backend->resizeSession(m_session, std::uint32_t(c.width()), std::uint32_t(c.height()));
	}
	refresh();
}

void TerminalView::applyToAll(const QJsonObject& settings)
{
	const QList<QWidget*> all = QApplication::allWidgets();
	for (QWidget* w : all)
		if (auto* t = dynamic_cast<TerminalView*>(w))
			t->applySettings(settings);
}

void TerminalView::wheelEvent(QWheelEvent* e)
{
	if (!(e->modifiers() & Qt::ControlModifier))
	{
		// Drei Zeilen je Raste (120); feine Räder und Touchpads liefern Bruchteile.
		m_scrollAcc += e->angleDelta().y();
		const int lines = m_scrollAcc / 40;
		if (lines != 0)
		{
			m_scrollAcc -= lines * 40;
			if (m_snap.value(QStringLiteral("alt_screen")).toBool())
			{
				// Bildschirm-Programme (less, top) haben keinen Rücklauf: Pfeiltasten senden.
				const int key = lines > 0 ? Qt::Key_Up : Qt::Key_Down;
				for (int i = 0; i < qAbs(lines); ++i)
					m_backend->keyEvent(m_session, std::uint32_t(key), 0, QString(), true);
			}
			else
			{
				m_selStart = m_selEnd = QPoint(-1, -1);
				m_backend->terminalScroll(m_session, lines);
				refresh();
			}
		}
		e->accept();
		return;
	}
	// Feine Räder liefern Bruchteile: erst bei einer ganzen Raste (120) ändern.
	m_wheel += e->angleDelta().y();
	const int steps = m_wheel / 120;
	if (steps != 0)
	{
		m_wheel -= steps * 120;
		applyToAll(QJsonDocument::fromJson(m_backend->terminalFontStep(steps).toUtf8()).object());
	}
	e->accept();
}

QSize TerminalView::cells() const
{
	return QSize(qMax(20, width() / m_cw), qMax(5, height() / m_ch));
}

void TerminalView::refresh()
{
	const QString json = m_backend->terminalJson(m_session);
	m_snap = QJsonDocument::fromJson(json.toUtf8()).object();
	// Selbstheilung: Hat das Gitter der Sitzung eine andere Größe als die Ansicht fasst (zum Beispiel
	// nach einer verlorenen Größenmeldung), wird die Größe erneut gemeldet; sonst wäre die letzte Zeile
	// angeschnitten oder ein Streifen leer. Höchstens drei Versuche hintereinander.
	const int rows = m_snap.value(QStringLiteral("rows")).toInt();
	const int cols = m_snap.value(QStringLiteral("cols")).toInt();
	const QSize want = cells();
	if (rows > 0 && cols > 0 && (rows != want.height() || cols != want.width()) && m_healTries < 3 && isVisible())
	{
		++m_healTries;
		m_lastSize = want;
		m_backend->resizeSession(m_session, std::uint32_t(want.width()), std::uint32_t(want.height()));
		m_snap = QJsonDocument::fromJson(m_backend->terminalJson(m_session).toUtf8()).object();
	}
	else if (rows == want.height() && cols == want.width())
		m_healTries = 0;
	update();
}

void TerminalView::resizeEvent(QResizeEvent*)
{
	QSize c = cells();
	if (c != m_lastSize)
	{
		m_lastSize = c;
		m_backend->resizeSession(m_session, std::uint32_t(c.width()), std::uint32_t(c.height()));
		refresh();
	}
}

void TerminalView::paintEvent(QPaintEvent*)
{
	QPainter p(this);
	const QColor bgScheme(m_snap.value(QStringLiteral("bg")).toString());
	const QColor fgScheme(m_snap.value(QStringLiteral("fg")).toString());
	p.fillRect(rect(), bgScheme.isValid() ? bgScheme : QColor(0, 0, 0));
	p.setFont(m_font);
	const QJsonArray lines = m_snap.value(QStringLiteral("lines")).toArray();
	QPoint a = m_selStart, b = m_selEnd;
	if (b.y() < a.y() || (b.y() == a.y() && b.x() < a.x()))
		std::swap(a, b);
	for (int row = 0; row < lines.size(); ++row)
	{
		int col = 0;
		for (const QJsonValue& rv : lines[row].toArray())
		{
			const QJsonArray run = rv.toArray();
			const QString text = run[0].toString();
			QColor fg(run[1].toString()), bg(run[2].toString());
			const int flags = run[3].toInt();
			QFont f = m_font;
			f.setBold(flags & 1);
			f.setItalic(flags & 2);
			f.setUnderline(flags & 4);
			p.setFont(f);
			// Zeichen einzeln setzen, damit breite Zeichen zwei Zellen belegen.
			int i = 0;
			while (i < text.size())
			{
				const int len = text.at(i).isHighSurrogate() && i + 1 < text.size() ? 2 : 1;
				const QString ch = text.mid(i, len);
				const int span = (flags & 8) ? 2 : 1;
				const bool sel = m_selStart.x() >= 0 &&
				                 (row > a.y() || (row == a.y() && col >= a.x())) &&
				                 (row < b.y() || (row == b.y() && col <= b.x()));
				QRect cell(col * m_cw, row * m_ch, m_cw * span, m_ch);
				p.fillRect(cell, sel ? (m_selection.isValid() ? m_selection : palette().highlight().color()) : bg);
				p.setPen(sel ? (m_selection.isValid() && fgScheme.isValid() ? fgScheme : palette().highlightedText().color()) : fg);
				p.drawText(cell.x(), cell.y() + m_ascent, ch);
				col += span;
				i += len;
			}
		}
	}
	const int back = m_snap.value(QStringLiteral("scroll_offset")).toInt();
	if (back > 0)
	{
		// Marke oben rechts: Anzeige liegt im Rücklauf.
		const QString text = m_backend->text(QStringLiteral("term.scrolled")).arg(back);
		QFont lf = m_font;
		lf.setBold(true);
		p.setFont(lf);
		const QRect tr = QFontMetrics(lf).boundingRect(text).adjusted(-8, -3, 8, 3);
		const QRect box(width() - tr.width() - 6, 4, tr.width(), tr.height());
		p.fillRect(box, QColor(0, 0, 0, 170));
		p.setPen(QColor(255, 255, 255));
		p.drawText(box, Qt::AlignCenter, text);
	}
	const QJsonArray cur = m_snap.value(QStringLiteral("cursor")).toArray();
	if (m_snap.value(QStringLiteral("cursor_visible")).toBool() && cur.size() == 2)
	{
		QRect c(cur[1].toInt() * m_cw, cur[0].toInt() * m_ch, m_cw, m_ch);
		QColor cc(m_snap.value(QStringLiteral("cursor_color")).toString());
		if (!cc.isValid())
			cc = QColor(229, 229, 229);
		if (hasFocus())
		{
			cc.setAlpha(160);
			p.fillRect(c, cc);
		}
		else
		{
			p.setPen(cc);
			p.drawRect(c.adjusted(0, 0, -1, -1));
		}
	}
}

void TerminalView::keyPressEvent(QKeyEvent* e)
{
	const auto mods = e->modifiers();
	// Strg+Einfg kopiert, Umschalt+Einfg fügt ein (wie in vielen Terminals und bei PuTTY).
	if (e->key() == Qt::Key_Insert && mods == Qt::ControlModifier)
	{
		copySelection();
		return;
	}
	if (e->key() == Qt::Key_Insert && mods == Qt::ShiftModifier)
	{
		pasteClipboard();
		return;
	}
	if ((mods & Qt::ShiftModifier) && !(mods & Qt::ControlModifier) && (e->key() == Qt::Key_PageUp || e->key() == Qt::Key_PageDown) &&
	    !m_snap.value(QStringLiteral("alt_screen")).toBool())
	{
		m_selStart = m_selEnd = QPoint(-1, -1);
		m_backend->terminalScrollPage(m_session, e->key() == Qt::Key_PageUp);
		refresh();
		return;
	}
	if ((mods & Qt::ControlModifier) && (mods & Qt::ShiftModifier))
	{
		if (e->key() == Qt::Key_V)
		{
			pasteClipboard();
			return;
		}
		if (e->key() == Qt::Key_C)
		{
			copySelection();
			return;
		}
	}
	m_backend->keyEvent(m_session, std::uint32_t(e->key()), std::uint32_t(mods), e->text(), true);
}

QPoint TerminalView::cellAt(const QPoint& p) const
{
	return QPoint(qMax(0, p.x() / m_cw), qMax(0, p.y() / m_ch));
}

void TerminalView::mousePressEvent(QMouseEvent* e)
{
	setFocus();
	if (e->button() == Qt::LeftButton)
	{
		const QPoint c = cellAt(e->pos());
		// Dritter Klick kurz nach dem Doppelklick: ganze Zeile.
		if (m_tripleClock.isValid() && m_tripleClock.elapsed() < QApplication::doubleClickInterval() && c == m_tripleCell)
		{
			m_tripleClock.invalidate();
			m_skipRelease = true;
			selectLine(c.y());
			return;
		}
		m_selStart = m_selEnd = c;
		update();
	}
	else if (e->button() == Qt::MiddleButton)
		pasteText(QGuiApplication::clipboard()->text(QClipboard::Selection));
}

void TerminalView::mouseMoveEvent(QMouseEvent* e)
{
	if (e->buttons() & Qt::LeftButton)
	{
		m_selEnd = cellAt(e->pos());
		update();
	}
}

void TerminalView::mouseReleaseEvent(QMouseEvent* e)
{
	if (e->button() != Qt::LeftButton)
		return;
	// Nach Doppel- oder Dreifachklick bleibt die dort gesetzte Auswahl stehen.
	if (m_skipRelease)
	{
		m_skipRelease = false;
		return;
	}
	m_selEnd = cellAt(e->pos());
	if (m_selEnd == m_selStart)
	{
		m_selStart = m_selEnd = QPoint(-1, -1);
		update();
		return;
	}
	finishSelection();
}

QString TerminalView::selectedText()
{
	if (m_selStart.x() < 0)
		return QString();
	QPoint a = m_selStart, b = m_selEnd;
	if (b.y() < a.y() || (b.y() == a.y() && b.x() < a.x()))
		std::swap(a, b);
	return m_backend->selectText(m_session, a.y(), a.x(), b.y(), b.x());
}

/// Auswahl abgeschlossen: in die X11-Auswahl, auf Wunsch auch in die Zwischenablage.
void TerminalView::finishSelection()
{
	const QString text = selectedText();
	if (!text.isEmpty())
	{
		if (QGuiApplication::clipboard()->supportsSelection())
			QGuiApplication::clipboard()->setText(text, QClipboard::Selection);
		if (s_copyOnSelect)
			QGuiApplication::clipboard()->setText(text);
	}
	update();
}

bool TerminalView::copySelection()
{
	const QString text = selectedText();
	if (text.isEmpty())
		return false;
	QGuiApplication::clipboard()->setText(text);
	if (QGuiApplication::clipboard()->supportsSelection())
		QGuiApplication::clipboard()->setText(text, QClipboard::Selection);
	return true;
}

void TerminalView::pasteClipboard()
{
	pasteText(QGuiApplication::clipboard()->text());
}

void TerminalView::pasteText(const QString& text)
{
	if (text.isEmpty())
		return;
	// Mehrere Zeilen: Zeilenumbrüche wirken wie die Eingabetaste, Befehle laufen sofort los.
	const int lines = text.trimmed().count(QLatin1Char('\n')) + 1;
	if (s_confirmPaste && lines > 1)
	{
		QMessageBox box(QMessageBox::Question, m_backend->text(QStringLiteral("term.paste.title")),
		                m_backend->text(QStringLiteral("term.paste.text")).arg(lines), QMessageBox::NoButton, this);
		auto* ok = box.addButton(m_backend->text(QStringLiteral("term.paste.ok")), QMessageBox::AcceptRole);
		box.addButton(m_backend->text(QStringLiteral("term.paste.cancel")), QMessageBox::RejectRole);
		auto* never = new QCheckBox(m_backend->text(QStringLiteral("term.paste.never")), &box);
		box.setCheckBox(never);
		box.setDefaultButton(ok);
		box.exec();
		if (never->isChecked())
		{
			const QJsonObject r = QJsonDocument::fromJson(m_backend->setTerminalSettings(QStringLiteral("{\"confirm_paste\":false}")).toUtf8()).object();
			applyToAll(r);
		}
		if (box.clickedButton() != ok)
			return;
	}
	m_backend->paste(m_session, text);
}

void TerminalView::selectAll()
{
	const int rows = m_snap.value(QStringLiteral("rows")).toInt();
	const int cols = m_snap.value(QStringLiteral("cols")).toInt();
	if (rows <= 0 || cols <= 0)
		return;
	m_selStart = QPoint(0, 0);
	m_selEnd = QPoint(cols - 1, rows - 1);
	finishSelection();
}

/// Zeichen je Spalte einer Zeile des Bildschirms (breite Zeichen belegen zwei Spalten).
QStringList TerminalView::rowChars(int row) const
{
	QStringList out;
	const QJsonArray lines = m_snap.value(QStringLiteral("lines")).toArray();
	if (row < 0 || row >= lines.size())
		return out;
	for (const QJsonValue& rv : lines[row].toArray())
	{
		const QJsonArray run = rv.toArray();
		const QString text = run[0].toString();
		const bool wide = run[3].toInt() & 8;
		int i = 0;
		while (i < text.size())
		{
			const int len = text.at(i).isHighSurrogate() && i + 1 < text.size() ? 2 : 1;
			const QString ch = text.mid(i, len);
			out << ch;
			if (wide)
				out << ch;
			i += len;
		}
	}
	return out;
}

void TerminalView::selectWordAt(const QPoint& c)
{
	const QStringList cols = rowChars(c.y());
	if (c.x() < 0 || c.x() >= cols.size())
		return;
	auto isWord = [](const QString& s) {
		if (s.isEmpty())
			return false;
		const QChar ch = s.at(0);
		return ch.isLetterOrNumber() || QStringLiteral("_-./~:@%+=?&#").contains(ch);
	};
	const bool word = isWord(cols[c.x()]);
	int a = c.x(), b = c.x();
	if (word)
	{
		while (a > 0 && isWord(cols[a - 1]))
			--a;
		while (b + 1 < cols.size() && isWord(cols[b + 1]))
			++b;
	}
	m_selStart = QPoint(a, c.y());
	m_selEnd = QPoint(b, c.y());
	finishSelection();
}

void TerminalView::selectLine(int row)
{
	const QStringList cols = rowChars(row);
	int last = cols.size() - 1;
	while (last > 0 && cols[last].trimmed().isEmpty())
		--last;
	m_selStart = QPoint(0, row);
	m_selEnd = QPoint(qMax(0, last), row);
	finishSelection();
}

void TerminalView::mouseDoubleClickEvent(QMouseEvent* e)
{
	if (e->button() != Qt::LeftButton)
		return;
	setFocus();
	const QPoint c = cellAt(e->pos());
	m_skipRelease = true;
	selectWordAt(c);
	m_tripleCell = c;
	m_tripleClock.start();
}

void TerminalView::contextMenuEvent(QContextMenuEvent* e)
{
	QMenu menu(this);
	auto* copy = menu.addAction(m_backend->text(QStringLiteral("term.menu.copy")));
	copy->setEnabled(m_selStart.x() >= 0 && !selectedText().isEmpty());
	connect(copy, &QAction::triggered, this, [this] { copySelection(); });
	auto* paste = menu.addAction(m_backend->text(QStringLiteral("term.menu.paste")));
	paste->setEnabled(!QGuiApplication::clipboard()->text().isEmpty());
	connect(paste, &QAction::triggered, this, [this] { pasteClipboard(); });
	menu.addSeparator();
	auto* all = menu.addAction(m_backend->text(QStringLiteral("term.menu.selectall")));
	connect(all, &QAction::triggered, this, [this] { selectAll(); });
	menu.exec(e->globalPos());
	e->accept();
}

// ---------- SessionPane ----------

SessionPane::SessionPane(RjBackend* backend, std::uint64_t session, const QString& title, const QString& protocol,
                         QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
  , m_session(session)
  , m_title(title)
{
	connect(&m_loadTick, &QTimer::timeout, this, &SessionPane::updateHeader);
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(2, 2, 2, 2);
	lay->setSpacing(2);
	m_header = new QLabel(title, this);
	m_header->setObjectName(QStringLiteral("paneHeader"));
	m_header->setMinimumHeight(28);
	m_notice = new QLabel(this);
	m_notice->setWordWrap(true);
	m_notice->setVisible(false);
	m_notice->setObjectName(QStringLiteral("paneNotice"));
	m_reconnect = new QPushButton(backend->text(QStringLiteral("session.reconnect")), this);
	m_reconnect->setVisible(false);
	m_reconnect->setMinimumHeight(36);
	connect(m_reconnect, &QPushButton::clicked, this, [this] { m_backend->reconnect(m_session); });
	m_security = new SecurityBar(backend, this);
	m_ask = new ActionBar(backend, this);
	// Kopfzeile: Name und Zustand, bei SSH außerdem die Makro-Schaltflächen.
	auto* headRow = new QWidget(this);
	auto* hl = new QHBoxLayout(headRow);
	hl->setContentsMargins(0, 0, 0, 0);
	hl->addWidget(m_header, 1);
	if (protocol == QLatin1String("ssh"))
	{
		m_macroState = new QLabel(headRow);
		m_macroState->hide();
		m_macroCancel = new QPushButton(backend->text(QStringLiteral("macro.cancel")), headRow);
		m_macroCancel->hide();
		m_macroNew = new QPushButton(backend->text(QStringLiteral("macro.new")), headRow);
		m_macroNew->setToolTip(backend->text(QStringLiteral("macro.tip.new")));
		m_macroRun = new QPushButton(backend->text(QStringLiteral("macro.run")), headRow);
		m_macroRun->setToolTip(backend->text(QStringLiteral("macro.tip.run")));
		auto* menu = new QMenu(m_macroRun);
		m_macroRun->setMenu(menu);
		connect(menu, &QMenu::aboutToShow, this, [this] { fillMacroMenu(); });
		hl->addWidget(m_macroState);
		hl->addWidget(m_macroCancel);
		hl->addWidget(m_macroNew);
		hl->addWidget(m_macroRun);
		connect(m_macroCancel, &QPushButton::clicked, this, [this] {
			m_backend->edit(QStringLiteral("{\"op\":\"cancel_macro\",\"session\":%1}").arg(m_session));
		});
		connect(m_macroNew, &QPushButton::clicked, this, [this] {
			// Neues Makro anlegen, diesem Ziel zuordnen und den Editor öffnen.
			const QJsonObject info = QJsonDocument::fromJson(m_backend->sessionInfo(m_session).toUtf8()).object();
			const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QStringLiteral("{\"op\":\"macro_new\"}")).toUtf8()).object();
			const QString id = r.value(QStringLiteral("macro")).toString();
			if (id.isEmpty())
				return;
			const QString target = info.value(QStringLiteral("target")).toString();
			if (!target.isEmpty())
				m_backend->edit(QStringLiteral("{\"op\":\"macro_link\",\"id\":\"%1\",\"node\":\"%2\"}").arg(id, target));
			if (m_openMacro)
				m_openMacro(id);
		});
	}
	if (protocol == QLatin1String("web") || protocol == QLatin1String("proxmox"))
	{
		// Web-Sitzung: Zurück, Vor, Neu laden in der Kopfzeile (Seite läuft im Worker).
		auto nav = [&](QPushButton*& b, const char* label, const char* tip, const char* action) {
			b = new QPushButton(backend->text(QString::fromLatin1(label)), headRow);
			b->setToolTip(backend->text(QString::fromLatin1(tip)));
			b->setAccessibleName(b->toolTip());
			const QString a = QString::fromLatin1(action);
			connect(b, &QPushButton::clicked, this, [this, a] {
				webCommand(QStringLiteral("{\"op\":\"nav\",\"action\":\"%1\"}").arg(a));
				if (m_frame)
					m_frame->setFocus();
			});
			hl->addWidget(b);
		};
		nav(m_back, "web.back", "web.tip.back", "back");
		nav(m_forward, "web.forward", "web.tip.forward", "forward");
		nav(m_reload, "web.reload", "web.tip.reload", "reload");
		m_back->setEnabled(false);
		m_forward->setEnabled(false);
	}
	// Schmale Kopfzeile: kleinere Schaltflächen und Schrift.
	m_header->setMinimumHeight(18);
	for (auto* b : headRow->findChildren<QPushButton*>())
	{
		b->setObjectName(QStringLiteral("paneBtn"));
		QFont bf = b->font();
		bf.setPointSizeF(bf.pointSizeF() * 0.9);
		b->setFont(bf);
	}
	lay->addWidget(headRow);
	lay->addWidget(m_notice);
	lay->addWidget(m_security);
	lay->addWidget(m_ask);
	lay->addWidget(m_reconnect);
	if (protocol == QLatin1String("ssh"))
	{
		m_terminal = new TerminalView(backend, session, this);
		lay->addWidget(m_terminal, 1);
		m_terminal->refresh();
	}
	else
	{
		m_frame = new FrameView(backend, session, this);
		m_frame->setWeb(m_back != nullptr);
		lay->addWidget(m_frame, 1);
	}
}

QWidget* SessionPane::view() const
{
	return m_terminal ? static_cast<QWidget*>(m_terminal) : static_cast<QWidget*>(m_frame);
}

void SessionPane::setState(const QString& state, const QString& detail)
{
	m_state = state;
	m_stateText = m_backend->text(QStringLiteral("state.") + state);
	if (!detail.isEmpty())
		m_stateText += QStringLiteral(": ") + detail;
	updateHeader();
	const bool down = state == QLatin1String("disconnected") || state == QLatin1String("failed") ||
	                  state == QLatin1String("crashed");
	m_reconnect->setVisible(down);
}

void SessionPane::updateHeader()
{
	QString text = m_title;
	if (!m_pageTitle.isEmpty())
		text += QStringLiteral(" · ") + m_pageTitle;
	text += QStringLiteral(" — ") + m_stateText;
	if (m_loading)
		text += QStringLiteral(" · ")
		  + m_backend->text(QStringLiteral("web.header.loading")).arg(m_loadClock.elapsed() / 1000).arg(m_progress);
	m_header->setText(text);
}

void SessionPane::webCommand(const QString& json)
{
	m_backend->edit(QStringLiteral("{\"op\":\"web_command\",\"session\":%1,\"cmd\":%2}").arg(m_session).arg(json));
}

void SessionPane::setWebStatus(const QJsonObject& e)
{
	m_pageTitle = e.value(QStringLiteral("title")).toString();
	const bool loading = e.value(QStringLiteral("loading")).toBool();
	m_progress = e.value(QStringLiteral("progress")).toInt();
	if (loading && !m_loading)
	{
		m_loadClock.start();
		m_loadTick.start(1000);
	}
	else if (!loading)
		m_loadTick.stop();
	m_loading = loading;
	if (m_back)
	{
		m_back->setEnabled(e.value(QStringLiteral("back")).toBool());
		m_forward->setEnabled(e.value(QStringLiteral("forward")).toBool());
	}
	updateHeader();
}

void SessionPane::showWebMenu(const QJsonObject& e)
{
	if (!m_frame)
		return;
	const int id = e.value(QStringLiteral("id")).toInt();
	const bool option = e.value(QStringLiteral("kind")).toString() == QLatin1String("option");
	// Das Menü ist ein Popup im Hauptfenster; die Seite läuft ohne eigenes Fenster im Worker.
	auto* menu = new QMenu(this);
	menu->setAttribute(Qt::WA_DeleteOnClose);
	const QJsonArray items = e.value(QStringLiteral("items")).toArray();
	for (int i = 0; i < items.size(); ++i)
	{
		const QJsonObject it = items.at(i).toObject();
		const QString action = it.value(QStringLiteral("action")).toString();
		if (!option && action.isEmpty())
		{
			menu->addSeparator();
			continue;
		}
		const QString label = option ? it.value(QStringLiteral("label")).toString() : m_backend->text(QStringLiteral("web.menu.") + action);
		QAction* a = menu->addAction(label);
		a->setEnabled(it.value(QStringLiteral("enabled")).toBool());
		if (option)
		{
			a->setCheckable(true);
			a->setChecked(it.value(QStringLiteral("selected")).toBool());
		}
		connect(a, &QAction::triggered, this, [this, id, i, action, option] {
			if (option)
				webCommand(QStringLiteral("{\"op\":\"menu\",\"id\":%1,\"index\":%2}").arg(id).arg(i));
			else
				webCommand(QStringLiteral("{\"op\":\"menu\",\"id\":0,\"action\":\"%1\"}").arg(action));
		});
	}
	if (option)
	{
		// Ohne Auswahl geschlossen: Auswahlliste im Worker ebenfalls schließen.
		connect(menu, &QMenu::aboutToHide, this, [this, id, menu] {
			if (!menu->activeAction())
				webCommand(QStringLiteral("{\"op\":\"menu\",\"id\":%1}").arg(id));
		});
	}
	const QPoint at = m_frame->toLocal(QPoint(e.value(QStringLiteral("x")).toInt(), e.value(QStringLiteral("y")).toInt()));
	menu->popup(m_frame->mapToGlobal(at));
}

void SessionPane::setNotice(const QString& text)
{
	// Der Hinweis erscheint in der Fußzeile des Hauptfensters, nicht in der Sitzung.
	m_noticeText = text;
}

void SessionPane::fillMacroMenu()
{
	QMenu* menu = m_macroRun->menu();
	menu->clear();
	const QJsonArray list = QJsonDocument::fromJson(m_backend->edit(QStringLiteral("{\"op\":\"session_macros\",\"session\":%1}").arg(m_session)).toUtf8()).array();
	for (const QJsonValue& v : list)
	{
		const QJsonObject m = v.toObject();
		const QString name = m.value(QStringLiteral("name")).toString();
		const QString origin = m.value(QStringLiteral("origin")).toString();
		const QString id = m.value(QStringLiteral("id")).toString();
		const QString text = origin.isEmpty() ? name : m_backend->text(QStringLiteral("macro.run.origin")).arg(name, origin);
		menu->addAction(text, this, [this, id, name] {
			const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QStringLiteral("{\"op\":\"run_macro\",\"session\":%1,\"macro\":\"%2\"}").arg(m_session).arg(id)).toUtf8()).object();
			if (r.contains(QStringLiteral("error")))
			{
				setMacroResult(r.value(QStringLiteral("error")).toString(), false);
				return;
			}
			m_macroState->setText(m_backend->text(QStringLiteral("macro.running")).arg(name));
			m_macroState->show();
			m_macroCancel->show();
			m_macroRun->setEnabled(false);
			if (m_terminal)
				m_terminal->setFocus();
		});
	}
	if (list.isEmpty())
		menu->addAction(m_backend->text(QStringLiteral("macro.run.none")))->setEnabled(false);
}

void SessionPane::setMacroResult(const QString& text, bool ok)
{
	if (!m_macroState)
		return;
	m_macroState->setText(text);
	// Fehler in der Warnfarbe, Erfolg in normaler Schrift.
	m_macroState->setObjectName(ok ? QString() : QStringLiteral("paneNotice"));
	m_macroState->style()->unpolish(m_macroState);
	m_macroState->style()->polish(m_macroState);
	m_macroState->setVisible(!text.isEmpty());
	m_macroCancel->hide();
	m_macroRun->setEnabled(true);
	// Meldung nach einer Weile wieder ausblenden.
	QPointer<QLabel> label(m_macroState);
	QTimer::singleShot(20000, this, [label, text] {
		if (label && label->text() == text)
			label->hide();
	});
}
