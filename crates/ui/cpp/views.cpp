#include "views.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonDocument>
#include <QtGui/QClipboard>
#include <QtGui/QFontDatabase>
#include <QtGui/QGuiApplication>
#include <QtGui/QKeyEvent>
#include <QtGui/QPainter>
#include <QtWidgets/QVBoxLayout>

// ---------- FrameView ----------

FrameView::FrameView(RjBackend* backend, std::uint64_t session, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
  , m_session(session)
{
	setFocusPolicy(Qt::StrongFocus);
	setMouseTracking(true);
	setAttribute(Qt::WA_OpaquePaintEvent);
	setMinimumSize(160, 120);
}

FrameView::~FrameView()
{
	if (m_file && m_map)
		m_file->unmap(m_map);
}

void FrameView::attach(const QString& path, int w, int h)
{
	if (m_file && m_map)
		m_file->unmap(m_map);
	m_map = nullptr;
	m_image = QImage();
	m_file = std::make_unique<QFile>(path);
	if (!m_file->open(QIODevice::ReadOnly))
		return;
	m_map = m_file->map(0, qint64(w) * h * 4);
	if (m_map)
		// BGRA im Speicher entspricht QImage::Format_RGB32 (little endian).
		m_image = QImage(m_map, w, h, w * 4, QImage::Format_RGB32);
	update();
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
	if (hasFocus())
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
	update();
}

// ---------- TerminalView ----------

TerminalView::TerminalView(RjBackend* backend, std::uint64_t session, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
  , m_session(session)
{
	setFocusPolicy(Qt::StrongFocus);
	setAttribute(Qt::WA_InputMethodEnabled);
	m_font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
	m_font.setPointSize(11);
	QFontMetrics fm(m_font);
	m_cw = qMax(1, fm.horizontalAdvance(QLatin1Char('M')));
	m_ch = qMax(1, fm.height());
	m_ascent = fm.ascent();
	setMinimumSize(m_cw * 20, m_ch * 5);
}

QSize TerminalView::cells() const
{
	return QSize(qMax(20, width() / m_cw), qMax(5, height() / m_ch));
}

void TerminalView::refresh()
{
	const QString json = m_backend->terminalJson(m_session);
	m_snap = QJsonDocument::fromJson(json.toUtf8()).object();
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
	p.fillRect(rect(), QColor(0, 0, 0));
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
				p.fillRect(cell, sel ? palette().highlight().color() : bg);
				p.setPen(sel ? palette().highlightedText().color() : fg);
				p.drawText(cell.x(), cell.y() + m_ascent, ch);
				col += span;
				i += len;
			}
		}
	}
	const QJsonArray cur = m_snap.value(QStringLiteral("cursor")).toArray();
	if (m_snap.value(QStringLiteral("cursor_visible")).toBool() && cur.size() == 2)
	{
		QRect c(cur[1].toInt() * m_cw, cur[0].toInt() * m_ch, m_cw, m_ch);
		if (hasFocus())
			p.fillRect(c, QColor(229, 229, 229, 160));
		else
		{
			p.setPen(QColor(229, 229, 229));
			p.drawRect(c.adjusted(0, 0, -1, -1));
		}
	}
}

void TerminalView::keyPressEvent(QKeyEvent* e)
{
	const auto mods = e->modifiers();
	if ((mods & Qt::ControlModifier) && (mods & Qt::ShiftModifier))
	{
		if (e->key() == Qt::Key_V)
		{
			m_backend->paste(m_session, QGuiApplication::clipboard()->text());
			return;
		}
		if (e->key() == Qt::Key_C)
		{
			if (m_selStart.x() >= 0)
				QGuiApplication::clipboard()->setText(m_backend->selectText(
				  m_session, m_selStart.y(), m_selStart.x(), m_selEnd.y(), m_selEnd.x()));
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
		m_selStart = m_selEnd = cellAt(e->pos());
		update();
	}
	else if (e->button() == Qt::MiddleButton)
		m_backend->paste(m_session, QGuiApplication::clipboard()->text(QClipboard::Selection));
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
	m_selEnd = cellAt(e->pos());
	if (m_selEnd == m_selStart)
	{
		m_selStart = m_selEnd = QPoint(-1, -1);
		update();
		return;
	}
	QPoint a = m_selStart, b = m_selEnd;
	if (b.y() < a.y() || (b.y() == a.y() && b.x() < a.x()))
		std::swap(a, b);
	const QString text = m_backend->selectText(m_session, a.y(), a.x(), b.y(), b.x());
	if (QGuiApplication::clipboard()->supportsSelection())
		QGuiApplication::clipboard()->setText(text, QClipboard::Selection);
	update();
}

// ---------- SessionPane ----------

SessionPane::SessionPane(RjBackend* backend, std::uint64_t session, const QString& title, const QString& protocol,
                         QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
  , m_session(session)
  , m_title(title)
{
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
	lay->addWidget(m_header);
	lay->addWidget(m_notice);
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
	QString label = m_backend->text(QStringLiteral("state.") + state);
	QString text = m_title + QStringLiteral(" — ") + label;
	if (!detail.isEmpty())
		text += QStringLiteral(": ") + detail;
	m_header->setText(text);
	const bool down = state == QLatin1String("disconnected") || state == QLatin1String("failed") ||
	                  state == QLatin1String("crashed");
	m_reconnect->setVisible(down);
}

void SessionPane::setNotice(const QString& text)
{
	m_notice->setText(text);
	m_notice->setVisible(!text.isEmpty());
}
