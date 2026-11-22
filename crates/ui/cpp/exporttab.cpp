#include "exporttab.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDate>
#include <QtCore/QDir>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtCore/QStandardPaths>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QFileDialog>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QVBoxLayout>

ExportTab::ExportTab(RjBackend* backend, std::function<QStringList()> selectedIds, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_selectedIds(std::move(selectedIds))
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setObjectName(QStringLiteral("exportTab"));
	auto* lay = new QVBoxLayout(this);
	auto* intro = new QLabel(T("export.intro"), this);
	intro->setWordWrap(true);
	lay->addWidget(intro);
	auto* form = new QFormLayout;
	m_format = new QComboBox(this);
	m_format->addItem(T("export.format.json"), QStringLiteral("json"));
	m_format->addItem(T("export.format.csv"), QStringLiteral("csv"));
	form->addRow(T("export.format"), m_format);
	m_scope = new QComboBox(this);
	m_scope->addItem(T("export.scope.all"), QStringLiteral("all"));
	m_scope->addItem(T("export.scope.selection"), QStringLiteral("selection"));
	form->addRow(T("export.scope"), m_scope);
	m_passwords = new QCheckBox(T("export.passwords"), this);
	form->addRow(QString(), m_passwords);
	m_zip = new QCheckBox(T("export.zip"), this);
	form->addRow(QString(), m_zip);
	m_zipPassword = new QLineEdit(this);
	m_zipPassword->setEchoMode(QLineEdit::Password);
	m_zipPassword2 = new QLineEdit(this);
	m_zipPassword2->setEchoMode(QLineEdit::Password);
	form->addRow(T("export.zip.password"), m_zipPassword);
	form->addRow(T("export.zip.password2"), m_zipPassword2);
	m_path = new QLineEdit(this);
	auto* browse = new QPushButton(T("export.browse"), this);
	auto* pathRow = new QWidget(this);
	auto* pl = new QHBoxLayout(pathRow);
	pl->setContentsMargins(0, 0, 0, 0);
	pl->addWidget(m_path, 1);
	pl->addWidget(browse);
	form->addRow(T("export.path"), pathRow);
	lay->addLayout(form);
	m_warning = new QLabel(T("export.warning"), this);
	m_warning->setWordWrap(true);
	m_warning->setObjectName(QStringLiteral("fieldError"));
	lay->addWidget(m_warning);
	m_go = new QPushButton(T("export.go"), this);
	auto* row = new QHBoxLayout;
	row->addWidget(m_go);
	row->addStretch(1);
	lay->addLayout(row);
	m_result = new QLabel(this);
	m_result->setWordWrap(true);
	m_result->setTextInteractionFlags(Qt::TextSelectableByMouse);
	lay->addWidget(m_result);
	lay->addStretch(1);

	connect(m_format, &QComboBox::currentIndexChanged, this, [this] { update(); });
	connect(m_passwords, &QCheckBox::toggled, this, [this] { update(); });
	connect(m_zip, &QCheckBox::toggled, this, [this] { update(); });
	connect(m_path, &QLineEdit::textEdited, this, [this] { m_pathEdited = true; });
	connect(browse, &QPushButton::clicked, this, [this] {
		const QString f = QFileDialog::getSaveFileName(this, m_backend->text(QStringLiteral("export.browse")), m_path->text().isEmpty() ? suggestedPath() : m_path->text(),
		                                               QString(), nullptr, QFileDialog::DontConfirmOverwrite);
		if (!f.isEmpty())
		{
			m_path->setText(f);
			m_pathEdited = true;
		}
	});
	connect(m_go, &QPushButton::clicked, this, [this] { run(); });
	update();
}

QString ExportTab::suggestedPath() const
{
	const QString home = m_backend->homeDir();
	const QString base = home.isEmpty() ? QStandardPaths::writableLocation(QStandardPaths::DocumentsLocation) : home;
	const QString ext = m_zip->isChecked() ? QStringLiteral("zip") : m_format->currentData().toString();
	return QDir(base).filePath(QStringLiteral("export/remotejoe-export-%1.%2").arg(QDate::currentDate().toString(Qt::ISODate), ext));
}

void ExportTab::update()
{
	const bool json = m_format->currentData().toString() == QLatin1String("json");
	const bool pw = m_passwords->isChecked();
	// JSON mit Kennwörtern gibt es nur als verschlüsseltes ZIP.
	if (json && pw)
		m_zip->setChecked(true);
	m_zip->setEnabled(!(json && pw));
	const bool zip = m_zip->isChecked();
	m_zipPassword->setEnabled(zip);
	m_zipPassword2->setEnabled(zip);
	m_warning->setVisible(pw);
	if (!m_pathEdited)
		m_path->setText(suggestedPath());
	else if (zip && !m_path->text().endsWith(QLatin1String(".zip"), Qt::CaseInsensitive))
		m_path->setText(m_path->text().section(QLatin1Char('.'), 0, -2) + QStringLiteral(".zip"));
}

void ExportTab::wipeSecrets()
{
	m_zipPassword->clear();
	m_zipPassword2->clear();
}

void ExportTab::run()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const bool zip = m_zip->isChecked();
	if (zip && m_zipPassword->text() != m_zipPassword2->text())
	{
		m_result->setText(T("export.zip.mismatch"));
		return;
	}
	QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("export") },
	                 { QStringLiteral("format"), m_format->currentData().toString() },
	                 { QStringLiteral("scope"), m_scope->currentData().toString() },
	                 { QStringLiteral("passwords"), m_passwords->isChecked() },
	                 { QStringLiteral("zip"), zip },
	                 { QStringLiteral("zip_password"), m_zipPassword->text() },
	                 { QStringLiteral("path"), m_path->text() } };
	if (m_scope->currentData().toString() == QLatin1String("selection"))
		cmd.insert(QStringLiteral("ids"), QJsonArray::fromStringList(m_selectedIds()));
	const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
	if (r.contains(QStringLiteral("error")))
	{
		m_result->setText(T("export.failed").arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	wipeSecrets();
	QString text = T("export.done").arg(r.value(QStringLiteral("targets")).toInt()).arg(r.value(QStringLiteral("folders")).toInt()).arg(r.value(QStringLiteral("path")).toString());
	if (r.value(QStringLiteral("without_credentials")).toInt() > 0)
		text += QLatin1Char(' ') + T("export.nocreds").arg(r.value(QStringLiteral("without_credentials")).toInt());
	if (r.value(QStringLiteral("passwords")).toBool())
		text += QLatin1Char(' ') + T(r.value(QStringLiteral("zip")).toBool() ? "export.pw.zip" : "export.pw.plain");
	m_result->setText(text);
	m_pathEdited = false;
	update();
	emit status(text);
}
