#include "titlebar.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QUrl>
#include <QtGui/QDesktopServices>
#include <QtGui/QIcon>
#include <QtGui/QMouseEvent>
#include <QtGui/QWindow>
#include <QtWidgets/QApplication>
#include <QtWidgets/QDialogButtonBox>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
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

TitleBar::TitleBar(QWidget* window, const QString& name, const QString& version, QWidget* parent)
  : QWidget(parent)
  , m_window(window)
{
	setObjectName(QStringLiteral("titleBar"));
	setFixedHeight(32);
	setAutoFillBackground(false);
	setStyleSheet(QStringLiteral("QWidget#titleBar { background: palette(button); border-bottom: 1px solid palette(mid); }"
	                             "QWidget#titleBar QLabel { background: transparent; }"
	                             "QToolButton#titleClose:hover { background: #c42b1c; }"));
	auto* l = new QHBoxLayout(this);
	l->setContentsMargins(8, 0, 4, 0);
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
	auto* min = windowButton(QStyle::SP_TitleBarMinButton, tr("Minimieren"), this);
	m_max = windowButton(QStyle::SP_TitleBarMaxButton, tr("Maximieren"), this);
	auto* close = windowButton(QStyle::SP_TitleBarCloseButton, tr("Schließen"), this);
	close->setObjectName(QStringLiteral("titleClose"));
	l->addWidget(min);
	l->addWidget(m_max);
	l->addWidget(close);
	connect(min, &QToolButton::clicked, m_window, &QWidget::showMinimized);
	connect(m_max, &QToolButton::clicked, this, [this] { toggleMaximized(); });
	connect(close, &QToolButton::clicked, m_window, &QWidget::close);
	updateState();
}

void TitleBar::updateState()
{
	const bool max = m_window->isMaximized();
	m_max->setIcon(style()->standardIcon(max ? QStyle::SP_TitleBarNormalButton : QStyle::SP_TitleBarMaxButton));
	m_max->setToolTip(max ? tr("Wiederherstellen") : tr("Maximieren"));
	m_max->setAccessibleName(m_max->toolTip());
}

void TitleBar::toggleMaximized()
{
	if (m_window->isMaximized())
		m_window->showNormal();
	else
		m_window->showMaximized();
	updateState();
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
		toggleMaximized();
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
