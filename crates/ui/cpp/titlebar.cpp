#include "titlebar.h"
#include "faicons.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QFile>
#include <QtCore/QFileInfo>
#include <QtCore/QUrl>
#include <QtGui/QFontDatabase>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QPlainTextEdit>
#include <QtCore/QRegularExpression>
#include <QtWidgets/QTextBrowser>
#include <QtGui/QDesktopServices>
#include <QtGui/QIcon>
#include <QtGui/QMouseEvent>
#include <QtGui/QPainter>
#include <QtGui/QPixmap>
#include <QtGui/QWindow>
#include <QtWidgets/QApplication>
#include <QtWidgets/QDialogButtonBox>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QMenu>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QStyle>
#include <QtWidgets/QToolButton>
#include <QtWidgets/QVBoxLayout>

const char* const kHelpUrl = "https://github.com/joeMJ/remotejoe#readme";

namespace {

enum class Glyph { Minimize, Maximize, Restore, Close };

/// Font-Awesome-Symbole in der Textfarbe des Farbschemas: Die Standardsymbole des Systemstils sind unter Windows
/// im dunklen Schema dunkelgrau auf dunkel und kaum zu sehen.
QIcon glyphIcon(Glyph g, const QColor& color, qreal dpr)
{
	const char* name = g == Glyph::Minimize ? "r-window-minimize" : g == Glyph::Maximize ? "r-window-maximize" : g == Glyph::Restore ? "r-window-restore" : "xmark";
	QPixmap pm(QSize(16, 16) * dpr);
	pm.setDevicePixelRatio(dpr);
	pm.fill(Qt::transparent);
	QPainter p(&pm);
	faPaint(&p, QRectF(2, 2, 12, 12), QString::fromLatin1(name), color);
	p.end();
	return QIcon(pm);
}

QToolButton* windowButton(const QString& name, QWidget* parent)
{
	auto* b = new QToolButton(parent);
	b->setAutoRaise(true);
	b->setFocusPolicy(Qt::NoFocus);
	b->setFixedSize(34, 26);
	b->setToolTip(name);
	b->setAccessibleName(name);
	return b;
}

} // namespace

BrandStrip::BrandStrip(QWidget* window, const QString& name, const QString& version, QWidget* parent)
  : QWidget(parent)
  , m_window(window)
{
	setObjectName(QStringLiteral("brandStrip"));
	auto* l = new QHBoxLayout(this);
	l->setContentsMargins(8, 0, 8, 0);
	l->setSpacing(6);
	auto* logo = new QLabel(this);
	QIcon icon(QStringLiteral(":/logo/remotejoe-32.png"));
	logo->setPixmap(icon.pixmap(20, 20));
	logo->setFixedSize(20, 20);
	logo->setAccessibleName(name);
	// „remotejoe“ kräftig, dahinter Ausgabe und Version klein und gedämpft („advanced 0.1.63“).
	QString product = name, edition;
	if (const int sp = name.indexOf(QLatin1Char(' ')); sp > 0)
	{
		product = name.left(sp);
		edition = name.mid(sp + 1);
	}
	auto* title = new QLabel(product, this);
	QFont f = title->font();
	f.setPointSizeF(f.pointSizeF() * 0.95);
	f.setWeight(QFont::DemiBold);
	title->setFont(f);
	auto* ver = new QLabel(edition.isEmpty() ? version : QStringLiteral("%1 %2").arg(edition, version), this);
	m_ver = ver;
	QFont vf = font();
	vf.setPointSizeF(vf.pointSizeF() * 0.82);
	ver->setFont(vf);
	applyVersionColor();
	l->addWidget(logo);
	l->addWidget(title);
	l->addWidget(ver);
	l->addStretch(1);
}

QSize BrandStrip::sizeHint() const
{
	QSize s = QWidget::sizeHint();
	if (minimumHeight() > 0)
		s.setHeight(minimumHeight());
	return s;
}

QSize WindowControls::sizeHint() const
{
	QSize s = QWidget::sizeHint();
	if (minimumHeight() > 0)
		s.setHeight(minimumHeight());
	return s;
}

void BrandStrip::applyVersionColor()
{
	if (!m_ver)
		return;
	// Nebentext-Farbe des aktuellen Farbschemas; der Stylesheet-Hintergrund der Leiste ist die Schaltflächenfarbe.
	QPalette p = m_ver->palette();
	p.setColor(QPalette::WindowText, qApp->palette().color(QPalette::PlaceholderText));
	m_ver->setPalette(p);
}

void BrandStrip::changeEvent(QEvent* e)
{
	QWidget::changeEvent(e);
	if (e->type() == QEvent::PaletteChange || e->type() == QEvent::ApplicationPaletteChange)
		applyVersionColor();
}

void toggleMaximized(QWidget* window)
{
	if (window->isMaximized())
		window->showNormal();
	else
		window->showMaximized();
}

void BrandStrip::mousePressEvent(QMouseEvent* e)
{
	if (e->button() == Qt::LeftButton)
	{
		if (auto* h = m_window->windowHandle())
			h->startSystemMove();
		e->accept();
		return;
	}
	QWidget::mousePressEvent(e);
}

void BrandStrip::mouseDoubleClickEvent(QMouseEvent* e)
{
	if (e->button() == Qt::LeftButton)
	{
		toggleMaximized(m_window);
		e->accept();
		return;
	}
	QWidget::mouseDoubleClickEvent(e);
}

WindowControls::WindowControls(QWidget* window, QWidget* parent)
  : QWidget(parent)
  , m_window(window)
{
	auto* l = new QHBoxLayout(this);
	l->setContentsMargins(0, 0, 4, 0);
	l->setSpacing(0);
	m_burger = new QToolButton(this);
	m_burger->setAutoRaise(true);
	m_burger->setFocusPolicy(Qt::NoFocus);
	m_burger->setFixedSize(34, 26);
	m_burger->setPopupMode(QToolButton::InstantPopup);
	m_burger->setToolTip(tr("Menü"));
	m_burger->setAccessibleName(m_burger->toolTip());
	m_burger->setStyleSheet(QStringLiteral("QToolButton::menu-indicator { image: none; }"));
	m_burger->hide();
	paintBurgerIcon();
	m_min = windowButton(tr("Minimieren"), this);
	m_max = windowButton(tr("Maximieren"), this);
	m_close = windowButton(tr("Schließen"), this);
	auto* min = m_min;
	auto* close = m_close;
	close->setObjectName(QStringLiteral("titleClose"));
	l->addWidget(m_burger);
	l->addWidget(min);
	l->addWidget(m_max);
	l->addWidget(close);
	connect(min, &QToolButton::clicked, m_window, &QWidget::showMinimized);
	connect(m_max, &QToolButton::clicked, this, [this] {
		toggleMaximized(m_window);
		updateState();
	});
	connect(close, &QToolButton::clicked, m_window, &QWidget::close);
	setStyleSheet(QStringLiteral("QToolButton#titleClose:hover { background: #c42b1c; }"));
	updateState();
}

void WindowControls::paintBurgerIcon()
{
	const qreal dpr = devicePixelRatioF();
	QPixmap pm(QSize(18, 18) * dpr);
	pm.setDevicePixelRatio(dpr);
	pm.fill(Qt::transparent);
	QPainter p(&pm);
	faPaint(&p, QRectF(2, 2, 14, 14), QStringLiteral("bars"), qApp->palette().color(QPalette::ButtonText));
	p.end();
	m_burger->setIcon(QIcon(pm));
}

void WindowControls::changeEvent(QEvent* e)
{
	QWidget::changeEvent(e);
	if (e->type() == QEvent::PaletteChange || e->type() == QEvent::ApplicationPaletteChange)
	{
		paintBurgerIcon();
		paintGlyphs();
	}
}

void WindowControls::paintGlyphs()
{
	if (!m_min || !m_max || !m_close)
		return;
	const QColor c = qApp->palette().color(QPalette::ButtonText);
	const qreal dpr = devicePixelRatioF();
	m_min->setIcon(glyphIcon(Glyph::Minimize, c, dpr));
	m_max->setIcon(glyphIcon(m_window->isMaximized() ? Glyph::Restore : Glyph::Maximize, c, dpr));
	m_close->setIcon(glyphIcon(Glyph::Close, c, dpr));
}

void WindowControls::setBurgerMenu(QMenu* menu)
{
	m_burger->setMenu(menu);
}

void WindowControls::setBurgerVisible(bool on)
{
	m_burger->setVisible(on);
}

void WindowControls::updateState()
{
	const bool max = m_window->isMaximized();
	paintGlyphs();
	m_max->setToolTip(max ? tr("Wiederherstellen") : tr("Maximieren"));
	m_max->setAccessibleName(m_max->toolTip());
}

TitleBar::TitleBar(QWidget* window, const QString& name, const QString& version, QWidget* parent)
  : QWidget(parent)
  , m_window(window)
{
	setObjectName(QStringLiteral("titleBar"));
	setFixedHeight(32);
	setAutoFillBackground(false);
	setStyleSheet(QStringLiteral("QWidget#titleBar { background: palette(button); border-bottom: 1px solid palette(mid); }"
	                             "QWidget#titleBar QLabel { background: transparent; }"));
	auto* l = new QHBoxLayout(this);
	l->setContentsMargins(0, 0, 0, 0);
	l->setSpacing(0);
	m_brand = new BrandStrip(window, name, version, this);
	m_controls = new WindowControls(window, this);
	l->addWidget(m_brand);
	l->addStretch(1);
	l->addWidget(m_controls);
}

void TitleBar::restoreParts()
{
	auto* l = static_cast<QHBoxLayout*>(layout());
	m_brand->setParent(this);
	m_controls->setParent(this);
	m_controls->setBurgerVisible(false);
	m_brand->setFixedHeight(QWIDGETSIZE_MAX);
	m_brand->setMinimumHeight(0);
	m_brand->setMaximumHeight(QWIDGETSIZE_MAX);
	m_controls->setMinimumHeight(0);
	m_controls->setMaximumHeight(QWIDGETSIZE_MAX);
	l->insertWidget(0, m_brand);
	l->addWidget(m_controls);
	m_brand->show();
	m_controls->show();
}

void TitleBar::updateState()
{
	m_controls->updateState();
}

void TitleBar::mousePressEvent(QMouseEvent* e)
{
	if (e->button() == Qt::LeftButton)
	{
		if (auto* h = m_window->windowHandle())
			h->startSystemMove();
		e->accept();
		return;
	}
	QWidget::mousePressEvent(e);
}

void TitleBar::mouseDoubleClickEvent(QMouseEvent* e)
{
	if (e->button() == Qt::LeftButton)
	{
		toggleMaximized(m_window);
		updateState();
		e->accept();
		return;
	}
	QWidget::mouseDoubleClickEvent(e);
}

AboutDialog::AboutDialog(RjBackend* backend, const QString& version, QWidget* parent)
  : QDialog(parent)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setWindowTitle(T("about.title"));
	setModal(true);
	setAttribute(Qt::WA_DeleteOnClose);
	auto* root = new QVBoxLayout(this);
	auto* top = new QHBoxLayout;
	auto* logo = new QLabel(this);
	logo->setPixmap(QIcon(QStringLiteral(":/logo/remotejoe-128.png")).pixmap(96, 96));
	logo->setAlignment(Qt::AlignTop);
	auto* text = new QLabel(this);
	text->setTextFormat(Qt::RichText);
	text->setTextInteractionFlags(Qt::TextBrowserInteraction);
	text->setOpenExternalLinks(true);
	text->setWordWrap(true);
	// Lizenz und Verweis: ohne Erweiterungen Apache 2.0 mit Quelltext auf GitHub; mit freigeschalteten Erweiterungen
	// die Nutzungsbedingungen. Die Texte zeigt remotejoe selbst (eingebettet), nicht über den Desktop – das schlug je
	// nach Umgebung still fehl.
	QString license = T("about.license");
	QString licenseKey = QStringLiteral("license");
	QString licenseButton = T("about.license.show");
	QString sourceLine = QStringLiteral("<p><a href=\"https://github.com/joeMJ/remotejoe\">%1</a></p>").arg(T("about.source"));
	QString pluginsLine;
	if (backend->modulesActive())
	{
		license = T("about.license.advanced");
		licenseKey = QStringLiteral("terms");
		licenseButton = T("about.terms");
		sourceLine.clear();
		// Geladene Erweiterungen mit Fassung (aus ihren Manifesten).
		QStringList names;
		const QJsonObject lj = QJsonDocument::fromJson(backend->lizenzJson().toUtf8()).object();
		for (const QJsonValue& e : lj.value(QStringLiteral("installiert")).toArray())
			if (const QJsonObject o = e.toObject(); o.value(QStringLiteral("geladen")).toBool())
				names << (o.value(QStringLiteral("name")).toString() + QLatin1Char(' ') + o.value(QStringLiteral("version")).toString()).toHtmlEscaped();
		pluginsLine = QStringLiteral("<p>%1</p>").arg(names.isEmpty() ? T("about.plugins.none") : T("about.plugins").arg(names.join(QStringLiteral(" · "))));
	}
	text->setText(QStringLiteral("<h2>%6</h2>"
	                             "<p><b>%1</b></p>"
	                             "<p>%2</p>"
	                             "<p>%3<br>%4</p>"
	                             "%8"
	                             "%7"
	                             "<p><small>Qt %5</small></p>")
	                  .arg(T("about.version").arg(version), T("about.description"), T("about.copyright"), license, QString::fromLatin1(qVersion()), backend->appEdition().toHtmlEscaped(), sourceLine, pluginsLine));
	top->addWidget(logo);
	top->addSpacing(12);
	top->addWidget(text, 1);
	root->addLayout(top);
	// Rechtstexte im eingebauten Leser: Lizenz bzw. Nutzungsbedingungen, Bibliotheken Dritter (Übersicht und Lizenztexte).
	auto* legal = new QHBoxLayout;
	auto* showLicense = new QPushButton(licenseButton, this);
	auto* showThird = new QPushButton(T("about.third_party.show"), this);
	legal->addWidget(showLicense);
	legal->addWidget(showThird);
	legal->addStretch(1);
	root->addLayout(legal);
	QStringList sections{ licenseKey };
	if (licenseKey == QLatin1String("license"))
		sections << QStringLiteral("notice");
	sections << QStringLiteral("third_party") << QStringLiteral("third_party_licenses");
	connect(showLicense, &QPushButton::clicked, this, [this, backend, sections, licenseKey] { (new LegalDialog(backend, sections, licenseKey, this))->show(); });
	connect(showThird, &QPushButton::clicked, this, [this, backend, sections] { (new LegalDialog(backend, sections, QStringLiteral("third_party"), this))->show(); });
	auto* buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
	buttons->button(QDialogButtonBox::Close)->setText(T("about.close"));
	connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::close);
	root->addWidget(buttons);
	setMinimumWidth(460);
}

LegalDialog::LegalDialog(RjBackend* backend, const QStringList& sections, const QString& start, QWidget* parent)
  : QDialog(parent)
  , m_backend(backend)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setWindowTitle(T("legal.title"));
	setAttribute(Qt::WA_DeleteOnClose);
	resize(900, 660);
	auto* root = new QVBoxLayout(this);
	auto* head = new QHBoxLayout;
	m_sections = new QComboBox(this);
	for (const QString& key : sections)
		m_sections->addItem(T((QByteArray("legal.sec.") + key.toLatin1()).constData()), key);
	head->addWidget(new QLabel(T("legal.section"), this));
	head->addWidget(m_sections, 1);
	m_search = new QLineEdit(this);
	m_search->setPlaceholderText(T("legal.search"));
	m_search->setClearButtonEnabled(true);
	auto* next = new QPushButton(T("legal.search.next"), this);
	m_found = new QLabel(this);
	m_found->setObjectName(QStringLiteral("rjMuted"));
	head->addWidget(m_search, 1);
	head->addWidget(next);
	head->addWidget(m_found);
	root->addLayout(head);
	m_text = new QTextBrowser(this);
	m_text->setOpenLinks(false);
	m_text->document()->setDocumentMargin(16);
	connect(m_text, &QTextBrowser::anchorClicked, this, &LegalDialog::openLink);
	root->addWidget(m_text, 1);
	auto* buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
	buttons->button(QDialogButtonBox::Close)->setText(T("about.close"));
	connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::close);
	root->addWidget(buttons);
	connect(m_sections, &QComboBox::currentIndexChanged, this, [this] { showSection(m_sections->currentData().toString()); });
	connect(next, &QPushButton::clicked, this, &LegalDialog::findNext);
	connect(m_search, &QLineEdit::returnPressed, this, &LegalDialog::findNext);
	connect(m_search, &QLineEdit::textEdited, this, [this] { m_found->clear(); });
	const int i = m_sections->findData(start);
	m_sections->setCurrentIndex(qMax(0, i));
	showSection(m_sections->currentData().toString());
}

QString LegalDialog::rawText(const QString& key) const
{
	QString text;
	if (key == QLatin1String("third_party_licenses"))
	{
		// Die Datei neben dem Programm hat im Windows-Paket zusätzlich die Lizenzen der mitgelieferten C-Bibliotheken;
		// fehlt sie (Start aus dem Bauordner), gilt der eingebettete Text.
		QFile f(QCoreApplication::applicationDirPath() + QStringLiteral("/THIRD_PARTY_LICENSES.txt"));
		if (f.open(QIODevice::ReadOnly))
			text = QString::fromUtf8(f.readAll());
	}
	if (text.isEmpty())
		text = m_backend->legalText(key);
	return text;
}

namespace
{
// Text als HTML: maskiert, Zeilenumbrüche bleiben, Adressen werden zu Links.
QString textToHtml(const QString& text)
{
	QString html = text.toHtmlEscaped();
	static const QRegularExpression url(QStringLiteral(R"((https?://[^\s<>"'()]+[^\s<>"'().,;:]))"));
	html.replace(url, QStringLiteral("<a href=\"\\1\">\\1</a>"));
	return QStringLiteral("<pre style=\"white-space: pre-wrap; font-family: monospace;\">%1</pre>").arg(html);
}
} // namespace

const QList<LegalDialog::Block>& LegalDialog::blocks()
{
	if (m_blocksLoaded)
		return m_blocks;
	m_blocksLoaded = true;
	// Aufbau: Trennlinie „=====“, Kopfzeile „NAME – gilt für: a 1.0, b 2.0“, Trennlinie, Text bis zur nächsten Linie.
	const QStringList lines = rawText(QStringLiteral("third_party_licenses")).split(QLatin1Char('\n'));
	auto isRule = [](const QString& l) { return l.size() > 20 && l.startsWith(QLatin1String("=====")); };
	for (int i = 0; i < lines.size(); ++i)
	{
		if (!isRule(lines[i]) || i + 2 >= lines.size() || !isRule(lines[i + 2]))
			continue;
		Block b;
		const QString head = lines[i + 1];
		const int sep = head.indexOf(QStringLiteral(" – gilt für: "));
		b.title = sep > 0 ? head.left(sep) : head;
		b.appliesTo = sep > 0 ? head.mid(sep + 13) : QString();
		int j = i + 3;
		QStringList body;
		while (j < lines.size() && !(isRule(lines[j]) && j + 2 < lines.size() && isRule(lines[j + 2])))
			body << lines[j++];
		b.text = body.join(QLatin1Char('\n')).trimmed();
		m_blocks << b;
		i = j - 1;
	}
	return m_blocks;
}

void LegalDialog::showBlock(int i)
{
	const QList<Block>& all = blocks();
	if (i < 0 || i >= all.size())
		return;
	const Block& b = all[i];
	m_text->setHtml(QStringLiteral("<p><a href=\"rj:index\">← %1</a></p><h2>%2</h2><p><b>%3</b> %4</p>%5")
	                  .arg(m_backend->text(QStringLiteral("legal.sec.third_party_licenses")).toHtmlEscaped(), b.title.toHtmlEscaped(),
	                       m_backend->text(QStringLiteral("legal.applies")).toHtmlEscaped(), b.appliesTo.toHtmlEscaped(), textToHtml(b.text)));
	m_text->moveCursor(QTextCursor::Start);
}

void LegalDialog::openLink(const QUrl& url)
{
	if (url.scheme() == QLatin1String("http") || url.scheme() == QLatin1String("https"))
	{
		QDesktopServices::openUrl(url);
		return;
	}
	if (url.scheme() != QLatin1String("rj"))
		return;
	const QString what = url.path();
	if (what == QLatin1String("index"))
	{
		m_sections->setCurrentIndex(qMax(0, m_sections->findData(QStringLiteral("third_party_licenses"))));
		showSection(QStringLiteral("third_party_licenses"));
		return;
	}
	if (what.startsWith(QLatin1String("block/")))
	{
		showBlock(what.mid(6).toInt());
		return;
	}
	if (what.startsWith(QLatin1String("pkg/")))
	{
		// Paket „name version“: der erste Lizenztext, der dafür gilt.
		const QString pkg = QUrl::fromPercentEncoding(what.mid(4).toUtf8());
		const QList<Block>& all = blocks();
		for (int i = 0; i < all.size(); ++i)
			if ((QStringLiteral(", ") + all[i].appliesTo + QStringLiteral(",")).contains(QStringLiteral(", ") + pkg + QStringLiteral(",")))
			{
				m_sections->blockSignals(true);
				m_sections->setCurrentIndex(qMax(0, m_sections->findData(QStringLiteral("third_party_licenses"))));
				m_sections->blockSignals(false);
				showBlock(i);
				return;
			}
		m_found->setText(m_backend->text(QStringLiteral("legal.search.none")));
	}
}

void LegalDialog::showSection(const QString& key)
{
	const QString text = rawText(key);
	m_found->clear();
	if (text.isEmpty())
	{
		m_text->setPlainText(m_backend->text(QStringLiteral("legal.missing")));
		return;
	}
	if (key == QLatin1String("third_party"))
	{
		// Markdown mit Tabelle: Paketname verweist auf crates.io, die Lizenz auf ihren Text im Leser.
		QStringList lines = text.split(QLatin1Char('\n'));
		for (QString& l : lines)
		{
			if (!l.startsWith(QLatin1String("| ")) || l.startsWith(QLatin1String("| Paket")))
				continue;
			const QStringList c = l.split(QLatin1Char('|'));
			if (c.size() < 5)
				continue;
			const QString name = c[1].trimmed(), ver = c[2].trimmed(), lic = c[3].trimmed();
			if (name.isEmpty() || name.startsWith(QLatin1Char('-')))
				continue;
			l = QStringLiteral("| [%1](https://crates.io/crates/%1/%2) | %2 | [%3](rj:pkg/%4) |")
			      .arg(name, ver, lic, QString::fromUtf8(QUrl::toPercentEncoding(name + QLatin1Char(' ') + ver)));
		}
		m_text->setMarkdown(lines.join(QLatin1Char('\n')));
	}
	else if (key == QLatin1String("third_party_licenses"))
	{
		// Verzeichnis der Lizenztexte; ein Klick zeigt den Text samt der Pakete, für die er gilt.
		QString html = QStringLiteral("<h2>%1</h2><table cellspacing=\"0\" cellpadding=\"5\">").arg(m_backend->text(QStringLiteral("legal.sec.third_party_licenses")).toHtmlEscaped());
		const QList<Block>& all = blocks();
		for (int i = 0; i < all.size(); ++i)
			html += QStringLiteral("<tr><td valign=\"top\" width=\"190\"><a href=\"rj:block/%1\">%2</a></td><td>%3</td></tr>").arg(i).arg(all[i].title.toHtmlEscaped(), all[i].appliesTo.toHtmlEscaped());
		html += QStringLiteral("</table>");
		if (all.isEmpty())
			html = textToHtml(text);
		m_text->setHtml(html);
	}
	else
		m_text->setHtml(textToHtml(text));
	m_text->moveCursor(QTextCursor::Start);
}

void LegalDialog::findNext()
{
	const QString q = m_search->text();
	if (q.isEmpty())
		return;
	if (m_text->find(q))
	{
		m_found->clear();
		return;
	}
	// Vom Anfang weitersuchen; nichts gefunden: sagen.
	m_text->moveCursor(QTextCursor::Start);
	m_found->setText(m_text->find(q) ? QString() : m_backend->text(QStringLiteral("legal.search.none")));
}
