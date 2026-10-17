#include "titlebar.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QUrl>
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

QToolButton* windowButton(QStyle::StandardPixmap icon, const QString& name, QWidget* parent)
{
	auto* b = new QToolButton(parent);
	b->setIcon(parent->style()->standardIcon(icon));
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
	ver->setFont(f);
	QPalette p = ver->palette();
	p.setColor(QPalette::WindowText, palette().color(QPalette::PlaceholderText));
	ver->setPalette(p);
	l->addWidget(logo);
	l->addWidget(title);
	l->addWidget(ver);
	l->addStretch(1);
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
	auto* min = windowButton(QStyle::SP_TitleBarMinButton, tr("Minimieren"), this);
	m_max = windowButton(QStyle::SP_TitleBarMaxButton, tr("Maximieren"), this);
	auto* close = windowButton(QStyle::SP_TitleBarCloseButton, tr("Schließen"), this);
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
	p.setPen(QPen(palette().color(QPalette::ButtonText), 2, Qt::SolidLine, Qt::RoundCap));
	for (int y : { 4, 9, 14 })
		p.drawLine(3, y, 15, y);
	p.end();
	m_burger->setIcon(QIcon(pm));
}

void WindowControls::changeEvent(QEvent* e)
{
	QWidget::changeEvent(e);
	if (e->type() == QEvent::PaletteChange)
		paintBurgerIcon();
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
	m_max->setIcon(style()->standardIcon(max ? QStyle::SP_TitleBarNormalButton : QStyle::SP_TitleBarMaxButton));
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
	text->setText(QStringLiteral("<h2>remotejoe</h2>"
	                             "<p><b>%1</b></p>"
	                             "<p>%2</p>"
	                             "<p>%3<br>%4</p>"
	                             "<p><a href=\"https://github.com/joeMJ/remotejoe\">%5</a><br>"
	                             "<a href=\"https://github.com/joeMJ/remotejoe/blob/main/THIRD_PARTY.md\">%6</a></p>"
	                             "<p><small>Qt %7</small></p>")
	                  .arg(T("about.version").arg(version), T("about.description"), T("about.copyright"), T("about.license"), T("about.source"), T("about.third_party"),
	                       QString::fromLatin1(qVersion())));
	top->addWidget(logo);
	top->addSpacing(12);
	top->addWidget(text, 1);
	root->addLayout(top);
	auto* buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
	buttons->button(QDialogButtonBox::Close)->setText(T("about.close"));
	connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::close);
	root->addWidget(buttons);
	setMinimumWidth(460);
}
