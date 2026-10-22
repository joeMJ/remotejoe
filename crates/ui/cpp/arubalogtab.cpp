#include "arubalogtab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonDocument>
#include <QtCore/QUuid>
#include <QtGui/QFontDatabase>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QInputDialog>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QVBoxLayout>

ArubaLogTab::ArubaLogTab(RjBackend* backend, const QJsonObject& subject, const QString& ip, const QString& where, const QString& kind, const QString& filter, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_subject(subject)
  , m_ip(ip)
  , m_where(where)
  , m_kind(kind)
  , m_tag(QUuid::createUuid().toString(QUuid::WithoutBraces))
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setProperty("editor", true);
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);
	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(this, &hin);
	auto* hh = new QHBoxLayout;
	auto* hv = new QVBoxLayout;
	auto* caption = new QLabel(title(), head);
	QFont tf = caption->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	caption->setFont(tf);
	hv->addWidget(caption);
	m_state = new QLabel(head);
	m_state->setObjectName(QStringLiteral("rjMuted"));
	m_state->setWordWrap(true);
	m_state->setTextInteractionFlags(Qt::TextSelectableByMouse);
	hv->addWidget(m_state);
	hh->addLayout(hv, 1);
	auto* close = new QPushButton(T("mass.close"), head);
	hh->addWidget(close, 0, Qt::AlignTop);
	hin->addLayout(hh);
	auto* fh = new QHBoxLayout;
	fh->addWidget(new QLabel(T("aruba.log.filter"), head));
	m_filter = new QLineEdit(filter, head);
	m_filter->setPlaceholderText(T("aruba.log.filter.ph"));
	m_filter->setClearButtonEnabled(true);
	fh->addWidget(m_filter, 1);
	m_reload = new QPushButton(T("aruba.log.reload"), head);
	m_reload->setDefault(true);
	fh->addWidget(m_reload);
	hin->addLayout(fh);
	lay->addWidget(head);

	QVBoxLayout* oin = nullptr;
	auto* oc = rjui::card(this, &oin);
	m_text = new QPlainTextEdit(oc);
	m_text->setReadOnly(true);
	m_text->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
	m_text->setLineWrapMode(QPlainTextEdit::NoWrap);
	oin->addWidget(m_text, 1);
	lay->addWidget(oc, 1);

	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_reload, &QPushButton::clicked, this, &ArubaLogTab::load);
	connect(m_filter, &QLineEdit::returnPressed, this, &ArubaLogTab::load);
	load();
}

QString ArubaLogTab::title() const
{
	return m_backend->text(QString::fromLatin1(m_kind == QLatin1String("security") ? "aruba.log.security" : "aruba.log.system")) + QStringLiteral(" · ") + m_where;
}

bool ArubaLogTab::askFilter(RjBackend* backend, QWidget* parent, const QString& kind, QString* filter)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	bool ok = false;
	QInputDialog dlg(parent);
	dlg.setWindowTitle(T(kind == QLatin1String("security") ? "aruba.log.security.filtered" : "aruba.log.system.filtered"));
	dlg.setLabelText(T("aruba.log.filter.ask"));
	dlg.setTextValue(*filter);
	dlg.setOkButtonText(T("aruba.log.show"));
	dlg.setCancelButtonText(T("aruba.write.cancel"));
	dlg.setMinimumWidth(420);
	ok = dlg.exec() == QDialog::Accepted;
	if (!ok || dlg.textValue().trimmed().isEmpty())
		return false;
	*filter = dlg.textValue().trimmed();
	return true;
}

void ArubaLogTab::load()
{
	QJsonObject cmd = m_subject;
	cmd.insert(QStringLiteral("op"), QStringLiteral("aruba_log"));
	cmd.insert(QStringLiteral("kind"), m_kind);
	cmd.insert(QStringLiteral("filter"), m_filter->text());
	cmd.insert(QStringLiteral("ip"), m_ip);
	cmd.insert(QStringLiteral("tag"), m_tag);
	const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(m_backend->text(QStringLiteral("aruba.error")).arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	m_reload->setEnabled(false);
	m_state->setText(m_backend->text(QStringLiteral("aruba.log.busy")));
}

void ArubaLogTab::handleResult(const QJsonObject& r)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	m_reload->setEnabled(true);
	if (r.value(QStringLiteral("error")).isString())
	{
		m_state->setText(T("aruba.error").arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	const QString text = r.value(QStringLiteral("text")).toString();
	m_text->setPlainText(text.trimmed().isEmpty() ? T("aruba.log.empty") : text);
	QStringList state;
	state << QStringLiteral("%1 · %2").arg(r.value(QStringLiteral("host")).toString(), r.value(QStringLiteral("command")).toString());
	state << T("aruba.at").arg(QDateTime::fromSecsSinceEpoch(r.value(QStringLiteral("at")).toInteger()).toString(QStringLiteral("dd.MM.yyyy HH:mm:ss")));
	if (!r.value(QStringLiteral("complete")).toBool(true))
		state << T("aruba.log.incomplete");
	if (r.value(QStringLiteral("note")).isString())
		state << r.value(QStringLiteral("note")).toString();
	m_state->setText(state.join(QStringLiteral(" · ")));
}
