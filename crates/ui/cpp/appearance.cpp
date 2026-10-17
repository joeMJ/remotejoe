#include "appearance.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDir>
#include <QtCore/QFile>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QUrl>
#include <QtGui/QDesktopServices>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QVBoxLayout>

namespace {

QJsonObject parse(const QString& s)
{
	return QJsonDocument::fromJson(s.toUtf8()).object();
}

// Beispiel für eine eigene Palette; fehlende Farben kommen aus dem hellen bzw. dunklen Standardschema.
const char* const kExample = R"({
  "name": "Mein Schema",
  "dark": true,
  "window": "#1e1e2e",
  "base": "#181825",
  "alternate": "#1e1e2e",
  "text": "#cdd6f4",
  "muted": "#a6adc8",
  "highlight": "#89b4fa",
  "highlight_text": "#11111b",
  "button": "#313244"
}
)";

} // namespace

AppearanceTab::AppearanceTab(RjBackend* backend, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setObjectName(QStringLiteral("appearanceTab"));
	auto* lay = new QVBoxLayout(this);
	auto* form = new QFormLayout;
	m_scheme = new QComboBox(this);
	m_scheme->setAccessibleName(T("appearance.scheme"));
	form->addRow(T("appearance.scheme"), m_scheme);
	m_focus = new QCheckBox(T("appearance.focus"), this);
	form->addRow(QString(), m_focus);
	lay->addLayout(form);
	auto* help = new QLabel(T("appearance.help"), this);
	help->setWordWrap(true);
	lay->addWidget(help);
	auto* row = new QHBoxLayout;
	auto* open = new QPushButton(T("appearance.folder"), this);
	auto* reloadButton = new QPushButton(T("appearance.reload"), this);
	row->addWidget(open);
	row->addWidget(reloadButton);
	row->addStretch(1);
	lay->addLayout(row);
	m_notes = new QLabel(this);
	m_notes->setWordWrap(true);
	m_notes->setTextInteractionFlags(Qt::TextSelectableByMouse);
	lay->addWidget(m_notes);
	lay->addStretch(1);
	connect(m_scheme, &QComboBox::currentIndexChanged, this, [this] { apply(); });
	connect(m_focus, &QCheckBox::toggled, this, [this] { apply(); });
	connect(reloadButton, &QPushButton::clicked, this, [this] { reload(); });
	connect(open, &QPushButton::clicked, this, [this] {
		if (m_dir.isEmpty())
			return;
		QDir().mkpath(m_dir);
		// Beim ersten Öffnen eine Beispieldatei anlegen (nie eine vorhandene überschreiben).
		if (QDir(m_dir).entryList({ QStringLiteral("*.json") }).isEmpty())
		{
			QFile f(QDir(m_dir).filePath(QStringLiteral("beispiel.json")));
			if (f.open(QIODevice::WriteOnly | QIODevice::NewOnly))
				f.write(kExample);
		}
		QDesktopServices::openUrl(QUrl::fromLocalFile(m_dir));
		reload();
	});
	reload();
}

void AppearanceTab::reload()
{
	const QJsonObject a = parse(m_backend->appearanceJson());
	m_loading = true;
	m_scheme->clear();
	for (const QJsonValue& v : a.value(QStringLiteral("schemes")).toArray())
		m_scheme->addItem(v.toObject().value(QStringLiteral("label")).toString(), v.toObject().value(QStringLiteral("id")).toString());
	m_scheme->setCurrentIndex(qMax(0, m_scheme->findData(a.value(QStringLiteral("scheme")).toString())));
	m_focus->setChecked(a.value(QStringLiteral("focus_frame")).toBool(true));
	m_dir = a.value(QStringLiteral("dir")).toString();
	QStringList notes;
	for (const QJsonValue& n : a.value(QStringLiteral("notes")).toArray())
		notes << n.toString();
	m_notes->setText(notes.join(QLatin1Char('\n')));
	m_notes->setVisible(!notes.isEmpty());
	m_loading = false;
}

void AppearanceTab::apply()
{
	if (m_loading)
		return;
	QJsonObject o;
	o.insert(QStringLiteral("scheme"), m_scheme->currentData().toString());
	o.insert(QStringLiteral("focus_frame"), m_focus->isChecked());
	const QJsonObject r = parse(m_backend->setAppearance(QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Compact))));
	if (r.contains(QStringLiteral("error")))
		emit status(r.value(QStringLiteral("error")).toString());
	emit appearanceChanged();
}
