#include "settingstab.h"

#include "appearance.h"
#include "cards.h"
#include "kbcapture.h"
#include "views.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QScrollArea>
#include <QtWidgets/QSpinBox>
#include <QtWidgets/QStackedWidget>
#include <QtWidgets/QVBoxLayout>

namespace
{
QJsonObject parseObj(const QString& s)
{
	return QJsonDocument::fromJson(s.toUtf8()).object();
}

/// Bildlaufflächen in einer Karte: Hintergrund der Karte statt des Fensters.
void cardBackground(QWidget* w)
{
	w->setAutoFillBackground(false);
	QList<QScrollArea*> all = w->findChildren<QScrollArea*>();
	if (auto* self = qobject_cast<QScrollArea*>(w))
		all.prepend(self);
	for (auto* s : all)
	{
		s->setFrameShape(QFrame::NoFrame);
		s->viewport()->setAutoFillBackground(false);
		if (s->widget())
			s->widget()->setAutoFillBackground(false);
	}
}

/// Gedämpfter Hinweistext unter einem Feld.
QLabel* hint(const QString& text, QWidget* parent)
{
	auto* l = new QLabel(text, parent);
	l->setObjectName(QStringLiteral("rjMuted"));
	l->setWordWrap(true);
	return l;
}
} // namespace

SettingsTab::SettingsTab(RjBackend* backend, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
{
	setObjectName(QStringLiteral("settingsTab"));
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);

	// Kopfkarte: was hier eingestellt wird.
	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(this, &hin);
	auto* title = new QLabel(T("settings.title"), head);
	QFont tf = title->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	title->setFont(tf);
	hin->addWidget(title);
	hin->addWidget(hint(T("settings.sub"), head));
	lay->addWidget(head);

	// Links die Bereiche, rechts die Karte des gewählten Bereichs.
	auto* body = new QHBoxLayout;
	body->setSpacing(10);
	QVBoxLayout* ain = nullptr;
	auto* areas = rjui::card(this, &ain);
	areas->setFixedWidth(300);
	ain->addWidget(rjui::section(T("edit.areas"), areas));
	m_areaList = ain;
	QVBoxLayout* pin = nullptr;
	auto* pageCard = rjui::card(this, &pin);
	m_pageTitle = rjui::section(QString(), pageCard);
	pin->addWidget(m_pageTitle);
	m_pages = new QStackedWidget(pageCard);
	pin->addWidget(m_pages, 1);
	body->addWidget(areas, 0);
	body->addWidget(pageCard, 1);
	lay->addLayout(body, 1);

	m_loading = true;
	addArea(QStringLiteral("lock"), T("settings.lock"), pageLock(), true);
	addArea(QStringLiteral("keys"), T("settings.keys"), pageKeys(), true);
	addArea(QStringLiteral("terminal"), T("term.title"), new TerminalSettingsTab(m_backend), true);
	m_appearance = new AppearanceTab(m_backend);
	connect(m_appearance, &AppearanceTab::appearanceChanged, this, &SettingsTab::appearanceChanged);
	connect(m_appearance, &ToolTab::status, this, &ToolTab::status);
	addArea(QStringLiteral("appearance"), T("appearance.title"), m_appearance, false);
	m_snmp = new SnmpTab(m_backend);
	connect(m_snmp, &SnmpTab::status, this, [this](const QString& s) {
		refreshSummaries();
		emit status(s);
	});
	addArea(QStringLiteral("snmp"), T("snmp.title"), m_snmp, false);
	ain->addStretch(1);
	m_loading = false;
	reload();
	showSection(QStringLiteral("lock"));
}

void SettingsTab::addArea(const QString& key, const QString& title, QWidget* page, bool scroll)
{
	QWidget* holder = page;
	if (scroll)
	{
		auto* s = new QScrollArea(m_pages);
		s->setWidgetResizable(true);
		s->setWidget(page);
		holder = s;
	}
	cardBackground(holder);
	m_pages->addWidget(holder);
	m_titles.insert(key, title);

	auto* row = new rjui::ClickRow;
	auto* h = new QHBoxLayout(row);
	h->setContentsMargins(8, 6, 8, 6);
	auto* v = new QVBoxLayout;
	v->setSpacing(1);
	auto* t = new QLabel(title, row);
	QFont bf = t->font();
	bf.setBold(true);
	t->setFont(bf);
	auto* sum = new QLabel(row);
	sum->setObjectName(QStringLiteral("rjMuted"));
	sum->setWordWrap(true);
	v->addWidget(t);
	v->addWidget(sum);
	h->addLayout(v, 1);
	m_areaList->addWidget(row);
	m_areas.insert(key, Area{ row, sum, m_pages->count() - 1 });
	connect(row, &rjui::ClickRow::clicked, this, [this, key] { showSection(key); });
}

void SettingsTab::showSection(const QString& key)
{
	if (!m_areas.contains(key))
		return;
	m_current = key;
	m_pages->setCurrentIndex(m_areas.value(key).page);
	m_pageTitle->setText(m_titles.value(key).toUpper());
	for (auto it = m_areas.cbegin(); it != m_areas.cend(); ++it)
		it->row->setSelected(it.key() == key);
}

QString SettingsTab::edit(const QJsonObject& cmd)
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

QWidget* SettingsTab::pageLock()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	auto* f = new QFormLayout;
	m_idle = new QComboBox(w);
	m_idle->addItem(T("idle.off"), 0);
	for (int m : { 5, 10, 15, 30, 60 })
		m_idle->addItem(T("idle.minutes").arg(m), m);
	f->addRow(T("settings.idle"), m_idle);
	m_disconnect = new QCheckBox(T("bar.idle.disconnect"), w);
	f->addRow(QString(), m_disconnect);
	f->addRow(QString(), hint(T("settings.lock.hint"), w));
	m_clipSecs = new QSpinBox(w);
	m_clipSecs->setRange(0, 3600);
	m_clipSecs->setSingleStep(5);
	m_clipSecs->setSuffix(T("settings.clip.suffix"));
	m_clipSecs->setSpecialValueText(T("settings.clip.never"));
	f->addRow(T("settings.clip"), m_clipSecs);
	f->addRow(QString(), hint(T("clip.clear.text"), w));
	v->addLayout(f);
	v->addStretch(1);

	auto saveIdle = [this] {
		if (m_loading)
			return;
		const int mins = m_idle->currentData().toInt();
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") },
		       { QStringLiteral("lock_after_idle_mins"), mins },
		       { QStringLiteral("disconnect_on_lock"), m_disconnect->isChecked() } });
		emit status(rjText(m_backend, "status.idle").arg(mins == 0 ? rjText(m_backend, "idle.off") : rjText(m_backend, "idle.minutes").arg(mins)));
		refreshSummaries();
	};
	connect(m_idle, &QComboBox::currentIndexChanged, this, saveIdle);
	connect(m_disconnect, &QCheckBox::toggled, this, saveIdle);
	connect(m_clipSecs, &QSpinBox::valueChanged, this, [this](int secs) {
		if (m_loading)
			return;
		m_backend->setClipboardClearSecs(secs);
		refreshSummaries();
	});
	return w;
}

QWidget* SettingsTab::pageKeys()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	auto* f = new QFormLayout;
	m_keyCapture = new QCheckBox(T("settings.keycapture"), w);
	f->addRow(QString(), m_keyCapture);
	f->addRow(QString(), hint(T("settings.keycapture.hint"), w));
	m_closeTab = new QComboBox(w);
	for (const char* mode : { "outside", "always", "never" })
		m_closeTab->addItem(rjText(m_backend, (QByteArray("settings.closetab.") + mode).constData()), QString::fromLatin1(mode));
	f->addRow(T("settings.closetab"), m_closeTab);
	f->addRow(QString(), hint(T("settings.closetab.hint"), w));
	v->addLayout(f);
	v->addStretch(1);

	connect(m_keyCapture, &QCheckBox::toggled, this, [this](bool on) {
		if (m_loading)
			return;
		FrameView::s_keyCapture = on;
		if (!on)
			KeyCapture::release();
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("keyboard_capture"), on } });
		emit status(rjText(m_backend, on ? "keys.on" : "keys.off"));
		refreshSummaries();
	});
	connect(m_closeTab, &QComboBox::currentIndexChanged, this, [this] {
		if (m_loading)
			return;
		const QString mode = m_closeTab->currentData().toString();
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("close_tab_key"), mode } });
		emit closeTabKeyChanged(mode);
		refreshSummaries();
	});
	return w;
}

void SettingsTab::reload()
{
	m_loading = true;
	const QJsonObject s = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("settings") } }));
	if (const int i = m_idle->findData(s.value(QStringLiteral("lock_after_idle_mins")).toInt()); i >= 0)
		m_idle->setCurrentIndex(i);
	m_disconnect->setChecked(s.value(QStringLiteral("disconnect_on_lock")).toBool());
	m_clipSecs->setValue(m_backend->clipboardClearSecs());
	m_keyCapture->setChecked(FrameView::s_keyCapture);
	if (const int i = m_closeTab->findData(s.value(QStringLiteral("close_tab_key")).toString(QStringLiteral("outside"))); i >= 0)
		m_closeTab->setCurrentIndex(i);
	if (m_appearance)
		m_appearance->reload();
	if (m_snmp)
		m_snmp->reload();
	m_loading = false;
	refreshSummaries();
}

void SettingsTab::refreshSummaries()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto set = [this](const char* key, const QString& text) {
		if (auto it = m_areas.find(QString::fromLatin1(key)); it != m_areas.end())
			it->summary->setText(text);
	};
	const int mins = m_idle->currentData().toInt();
	const int secs = m_clipSecs->value();
	set("lock", T("settings.lock.sum")
	              .arg(mins == 0 ? T("idle.off") : T("idle.minutes").arg(mins))
	              .arg(secs == 0 ? T("settings.clip.never") : QString::number(secs) + T("settings.clip.suffix")));
	set("keys", T(m_keyCapture->isChecked() ? "settings.keys.sum.on" : "settings.keys.sum.off"));
	set("terminal", T("settings.terminal.sum"));
	set("appearance", T("settings.appearance.sum"));
	const int devices = int(QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("snmp_list") } }).toUtf8()).array().size());
	set("snmp", T("settings.snmp.sum").arg(devices));
}

void SettingsTab::wipeSecrets()
{
	if (m_snmp)
		m_snmp->wipeSecrets();
}
