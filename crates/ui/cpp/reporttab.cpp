#include "reporttab.h"

#include "cards.h"
#include "diag.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtCore/QSysInfo>
#include <QtCore/QUrl>
#include <QtGui/QClipboard>
#include <QtGui/QDesktopServices>
#include <QtGui/QGuiApplication>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QVBoxLayout>

ReportTab::ReportTab(RjBackend* backend, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setProperty("editor", true);
	setObjectName(QStringLiteral("reportTab"));
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);

	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(this, &hin);
	auto* hh = new QHBoxLayout;
	auto* hv = new QVBoxLayout;
	auto* caption = new QLabel(T("report.head"), head);
	QFont tf = caption->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	caption->setFont(tf);
	hv->addWidget(caption);
	m_state = new QLabel(T("report.intro"), head);
	m_state->setObjectName(QStringLiteral("rjMuted"));
	m_state->setWordWrap(true);
	hv->addWidget(m_state);
	hh->addLayout(hv, 1);
	m_open = new QPushButton(T("report.open"), head);
	m_open->setDefault(true);
	auto* close = new QPushButton(T("mass.close"), head);
	hh->addWidget(m_open, 0, Qt::AlignTop);
	hh->addWidget(close, 0, Qt::AlignTop);
	hin->addLayout(hh);
	lay->addWidget(head);

	QVBoxLayout* fin = nullptr;
	auto* card = rjui::card(this, &fin);
	auto* form = new QFormLayout;
	m_kind = new QComboBox(card);
	m_kind->addItem(T("report.kind.bug"), QStringLiteral("bug"));
	m_kind->addItem(T("report.kind.wish"), QStringLiteral("wish"));
	m_title = new QLineEdit(card);
	m_title->setPlaceholderText(T("report.title.ph"));
	m_text = new QPlainTextEdit(card);
	m_text->setPlaceholderText(T("report.text.ph"));
	m_diag = new QCheckBox(T("report.diag"), card);
	m_diag->setChecked(true);
	form->addRow(T("report.kind"), m_kind);
	form->addRow(T("report.title"), m_title);
	form->addRow(T("report.text"), m_text);
	form->addRow(QString(), m_diag);
	fin->addLayout(form, 1);
	fin->addWidget(rjui::section(T("report.preview"), card));
	m_preview = new QPlainTextEdit(card);
	m_preview->setReadOnly(true);
	fin->addWidget(m_preview, 1);
	lay->addWidget(card, 1);

	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_open, &QPushButton::clicked, this, &ReportTab::open);
	connect(m_kind, &QComboBox::currentIndexChanged, this, &ReportTab::preview);
	connect(m_title, &QLineEdit::textChanged, this, &ReportTab::preview);
	connect(m_text, &QPlainTextEdit::textChanged, this, &ReportTab::preview);
	connect(m_diag, &QCheckBox::toggled, this, &ReportTab::preview);
	preview();
}

QJsonObject ReportTab::build() const
{
	QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("report_build") },
		             { QStringLiteral("kind"), m_kind->currentData().toString() },
		             { QStringLiteral("title"), m_title->text() },
		             { QStringLiteral("text"), m_text->toPlainText() },
		             { QStringLiteral("version"), m_backend->appVersion() },
		             { QStringLiteral("os"), QSysInfo::prettyProductName() + QStringLiteral(" (") + QSysInfo::currentCpuArchitecture() + QStringLiteral(")") } };
	if (m_diag->isChecked())
		cmd.insert(QStringLiteral("diag"), QJsonArray::fromStringList(rjdiag::recent(40)));
	return QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
}

void ReportTab::preview()
{
	const QJsonObject r = build();
	m_preview->setPlainText(r.value(QStringLiteral("title")).toString() + QStringLiteral("\n\n") + r.value(QStringLiteral("body")).toString());
	m_open->setEnabled(!m_title->text().trimmed().isEmpty());
}

void ReportTab::open()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const QJsonObject r = build();
	if (r.value(QStringLiteral("cut")).toBool())
	{
		// Zu lang für die Adresse: vollständiger Text in die Zwischenablage, zum Einfügen auf GitHub.
		QGuiApplication::clipboard()->setText(r.value(QStringLiteral("body")).toString());
		m_state->setText(T("report.cut"));
	}
	else
		m_state->setText(T("report.opened"));
	QDesktopServices::openUrl(QUrl(r.value(QStringLiteral("url")).toString(), QUrl::StrictMode));
}
