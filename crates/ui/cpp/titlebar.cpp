#include "titlebar.h"

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

/// Eigene Symbole in der Textfarbe des Farbschemas: Die Standardsymbole des Systemstils sind unter Windows im
/// dunklen Schema dunkelgrau auf dunkel und kaum zu sehen.
QIcon glyphIcon(Glyph g, const QColor& color, qreal dpr)
{
	QPixmap pm(QSize(16, 16) * dpr);
	pm.setDevicePixelRatio(dpr);
	pm.fill(Qt::transparent);
	QPainter p(&pm);
	p.setPen(QPen(color, 1.3, Qt::SolidLine, Qt::SquareCap, Qt::MiterJoin));
	switch (g)
	{
	case Glyph::Minimize:
		p.drawLine(QPointF(3.5, 11.5), QPointF(12.5, 11.5));
		break;
	case Glyph::Maximize:
		p.drawRect(QRectF(3.5, 3.5, 9, 9));
		break;
	case Glyph::Restore:
		p.drawRect(QRectF(3.5, 5.5, 7, 7));
		p.drawLine(QPointF(5.5, 3.5), QPointF(12.5, 3.5));
		p.drawLine(QPointF(12.5, 3.5), QPointF(12.5, 10.5));
		p.drawLine(QPointF(10.5, 10.5), QPointF(12.5, 10.5));
		break;
	case Glyph::Close:
		p.drawLine(QPointF(3.5, 3.5), QPointF(12.5, 12.5));
		p.drawLine(QPointF(12.5, 3.5), QPointF(3.5, 12.5));
		break;
	}
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
	auto* title = new QLabel(name, this);
	QFont f = title->font();
	f.setPointSizeF(f.pointSizeF() * 0.9);
	title->setFont(f);
	auto* ver = new QLabel(QStringLiteral("V %1").arg(version), this);
	m_ver = ver;
	ver->setFont(f);
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
	p.setPen(QPen(qApp->palette().color(QPalette::ButtonText), 2, Qt::SolidLine, Qt::RoundCap));
	for (int y : { 4, 9, 14 })
		p.drawLine(3, y, 15, y);
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
	m_text = new QPlainTextEdit(this);
	m_text->setReadOnly(true);
	m_text->setLineWrapMode(QPlainTextEdit::WidgetWidth);
	m_text->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
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

void LegalDialog::showSection(const QString& key)
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
	if (text.isEmpty())
		text = m_backend->text(QStringLiteral("legal.missing"));
	m_text->setPlainText(text);
	m_text->moveCursor(QTextCursor::Start);
	m_found->clear();
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
