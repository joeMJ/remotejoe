#include "settingstab.h"

#include "appearance.h"
#include "editor.h"
#include "cards.h"
#include "rjpluginui.h"

RjHostBackend* SettingsTab::hostBackend() const
{
	return rjHostBackend(m_backend);
}
#include "kbcapture.h"
#include "views.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDir>
#include <QtCore/QJsonArray>
#include <QtCore/QLocale>
#include <QtCore/QUrl>
#include <QtGui/QDesktopServices>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QFileDialog>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QKeySequenceEdit>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QListWidget>
#include <QtGui/QClipboard>
#include <QtGui/QGuiApplication>
#include <QtCore/QCoreApplication>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QScrollArea>
#include <QtWidgets/QDoubleSpinBox>
#include <QtWidgets/QSpinBox>
#include <QtGui/QStandardItemModel>
#include <QtWidgets/QStackedWidget>
#include <QtWidgets/QTreeWidget>
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
	// Schlanker Bereichsbaum mit Gruppen und Symbolen (rollt selbst bei vielen Bereichen).
	m_areaTree = new rjui::AreaTree(areas);
	m_areaTree->onSelect = [this](const QString& key, QTreeWidgetItem*) { showSection(key); };
	ain->addWidget(m_areaTree, 1);
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
	// Gruppen: Sicherheit, Darstellung, Daten, Funktionen der Erweiterungen, Lizenz, Claude.
	addGroupIcon(QStringLiteral("shield-halved"), T("settings.group.security"), 0);
	addArea(QStringLiteral("lock"), T("settings.lock.entry"), pageLock(), true, 1);
	addArea(QStringLiteral("keys"), T("settings.keys"), pageKeys(), true, 1);
	addArea(QStringLiteral("ciphers"), T("settings.ciphers"), pageCiphers(), true, 1);
	addGroupIcon(QStringLiteral("palette"), T("settings.group.look"), 0);
	m_appearance = new AppearanceTab(m_backend);
	connect(m_appearance, &AppearanceTab::appearanceChanged, this, &SettingsTab::appearanceChanged);
	connect(m_appearance, &ToolTab::status, this, &ToolTab::status);
	addArea(QStringLiteral("appearance"), T("appearance.title"), m_appearance, false, 1);
	addArea(QStringLiteral("terminal"), T("term.title"), new TerminalSettingsTab(m_backend), true, 1);
	// SNMP-Geräte: kein Bereich mehr in den Einstellungen (kommt später mit „Monitoring-Integration“). Ampeln im
	// Baum und Zuordnung zu Zielen bleiben im Kern.
	addGroupIcon(QStringLiteral("hard-drive"), T("settings.group.data"), 0);
	addArea(QStringLiteral("backup"), T("settings.backup"), pageBackup(), true, 1);
	addArea(QStringLiteral("webcache"), T("settings.webcache"), pageWebCache(), true, 1);
	addArea(QStringLiteral("diag"), T("settings.diag"), pageDiag(), true, 1);
	for (RjPluginUi* p : rjPluginUis(m_backend))
		if (RjSettingsPart* part = p->settingsPart(this))
			m_parts.push_back(part);
	for (RjSettingsPart* part : m_parts)
		part->addAreas();
	// Seite „Erweiterungen“ vor der Lizenzseite anlegen: die Lizenzseite füllt sie mit.
	QWidget* extPage = pageExtensions();
	addGroupIcon(QStringLiteral("puzzle-piece"), T("settings.group.license"), 0);
	addArea(QStringLiteral("license"), T("settings.license"), pageLicense(), true, 1);
	addGroupIcon(QStringLiteral("robot"), T("settings.group.claude"), 0);
	addArea(QStringLiteral("mcp"), T("settings.mcp"), pageMcp(), true, 1);
	// Updates: Programm (immer) und Erweiterungen (nur mit eingespielter Lizenz, siehe fillLicensePage).
	addGroupIcon(QStringLiteral("arrows-rotate"), T("settings.group.updates"), 0);
	{
		auto* up = new UpdateTab(m_backend);
		connect(up, &ToolTab::status, this, &ToolTab::status);
		addArea(QStringLiteral("updates_program"), T("settings.updates.program"), up, true, 1);
	}
	addArea(QStringLiteral("updates_ext"), T("settings.updates.ext"), extPage, true, 1);
	// Nur mit eingespielter Lizenz sichtbar.
	m_areas.value(QStringLiteral("updates_ext")).item->setHidden(
	  QJsonDocument::fromJson(m_backend->lizenzJson().toUtf8()).object().value(QStringLiteral("lizenz")).toObject().isEmpty());
	m_loading = false;
	reload();
	showSection(QStringLiteral("lock"));
}

void SettingsTab::addGroup(const QString& title, int indent)
{
	addGroupIcon(QStringLiteral("layer-group"), title, indent);
}

void SettingsTab::addGroupIcon(const QString& glyph, const QString& title, int indent)
{
	QTreeWidgetItem* parent = indent > 0 ? m_groupAt.value(indent - 1) : nullptr;
	m_groupAt.insert(indent, m_areaTree->addGroup(glyph, title, parent));
}

void SettingsTab::addArea(const QString& key, const QString& title, QWidget* page, bool scroll, int indent)
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
	// Symbol je Bereich; Bereiche der Erweiterungen bekommen ein allgemeines.
	static const QHash<QString, QString> glyphs = {
		{ QStringLiteral("lock"), QStringLiteral("lock") }, { QStringLiteral("keys"), QStringLiteral("keyboard") },
		{ QStringLiteral("ciphers"), QStringLiteral("key") },
		{ QStringLiteral("terminal"), QStringLiteral("terminal") }, { QStringLiteral("appearance"), QStringLiteral("palette") },
		{ QStringLiteral("backup"), QStringLiteral("clock-rotate-left") }, { QStringLiteral("webcache"), QStringLiteral("eraser") }, { QStringLiteral("diag"), QStringLiteral("bug") }, { QStringLiteral("license"), QStringLiteral("certificate") },
		{ QStringLiteral("mcp"), QStringLiteral("robot") },
		{ QStringLiteral("updates_program"), QStringLiteral("download") }, { QStringLiteral("updates_ext"), QStringLiteral("puzzle-piece") },
	};
	QTreeWidgetItem* parent = indent > 0 ? m_groupAt.value(indent - 1) : nullptr;
	QTreeWidgetItem* item = m_areaTree->addEntry(parent, key, glyphs.value(key, QStringLiteral("sliders")), QString(title).remove(QLatin1Char('&')));
	m_areas.insert(key, Area{ item, m_pages->count() - 1 });
}

void SettingsTab::showSection(const QString& key)
{
	if (!m_areas.contains(key))
		return;
	m_current = key;
	m_pages->setCurrentIndex(m_areas.value(key).page);
	m_pageTitle->setText(QString(m_titles.value(key)).remove(QLatin1Char('&')).toUpper());
	m_areaTree->selectKey(key);
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
	// Schlüsselfehler (Host-Schlüssel, Zertifikate): ohne Rückfrage nur für diese Verbindung annehmen.
	m_keyErrors = new QCheckBox(T("settings.keyerrors"), w);
	f->addRow(QString(), m_keyErrors);
	f->addRow(QString(), hint(T("settings.keyerrors.hint"), w));
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
	connect(m_keyErrors, &QCheckBox::toggled, this, [this](bool on) {
		if (m_loading)
			return;
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("accept_key_errors"), on } });
		refreshSummaries();
	});
	connect(m_clipSecs, &QSpinBox::valueChanged, this, [this](int secs) {
		if (m_loading)
			return;
		m_backend->setClipboardClearSecs(secs);
		refreshSummaries();
	});
	return w;
}

void SettingsTab::saveTabSwitchKeys()
{
	// Nur die erste Tastenkombination zählt (ältere Qt-Fassungen nehmen bis zu vier auf).
	auto first = [](QKeySequenceEdit* e) { return e->keySequence().isEmpty() ? QString() : QKeySequence(e->keySequence()[0]).toString(QKeySequence::PortableText); };
	const QString mode = m_tabSwitch->currentData().toString();
	const QString prev = first(m_tabPrev);
	const QString next = first(m_tabNext);
	edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("tab_switch_key"), mode }, { QStringLiteral("tab_prev_keys"), prev }, { QStringLiteral("tab_next_keys"), next } });
	emit tabSwitchKeysChanged(mode, prev, next);
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
	m_findKey = new QComboBox(w);
	for (const char* mode : { "outside", "always", "never" })
		m_findKey->addItem(rjText(m_backend, (QByteArray("settings.closetab.") + mode).constData()), QString::fromLatin1(mode));
	f->addRow(T("settings.findkey"), m_findKey);
	f->addRow(QString(), hint(T("settings.findkey.hint"), w));
	// Tab nach links/rechts wechseln: Modus wie bei Strg+Umschalt+W, Tastenfolgen frei belegbar.
	m_tabSwitch = new QComboBox(w);
	for (const char* mode : { "outside", "always", "never" })
		m_tabSwitch->addItem(rjText(m_backend, (QByteArray("settings.closetab.") + mode).constData()), QString::fromLatin1(mode));
	f->addRow(T("settings.tabswitch"), m_tabSwitch);
	auto keyEdit = [w] {
		auto* e = new QKeySequenceEdit(w);
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
		e->setMaximumSequenceLength(1);
		e->setClearButtonEnabled(true);
#endif
		return e;
	};
	m_tabPrev = keyEdit();
	m_tabNext = keyEdit();
	f->addRow(T("settings.tabswitch.prev"), m_tabPrev);
	f->addRow(T("settings.tabswitch.next"), m_tabNext);
	auto* tabReset = new QPushButton(T("settings.tabswitch.reset"), w);
	auto* tabResetRow = new QHBoxLayout;
	tabResetRow->addWidget(tabReset);
	tabResetRow->addStretch(1);
	f->addRow(QString(), tabResetRow);
	f->addRow(QString(), hint(T("settings.tabswitch.hint"), w));
	// Administrativer Tastaturmodus (RDP, je Sitzung über das Kontextmenü des Tabs).
	m_adminLayout = new QComboBox(w);
	f->addRow(T("settings.adminkeys.layout"), m_adminLayout);
	f->addRow(QString(), hint(T("settings.adminkeys.layout.hint"), w));
	m_adminTimeout = new QSpinBox(w);
	m_adminTimeout->setRange(0, 24 * 60);
	m_adminTimeout->setSuffix(T("settings.adminkeys.timeout.suffix"));
	m_adminTimeout->setSpecialValueText(T("settings.adminkeys.timeout.never"));
	f->addRow(T("settings.adminkeys.timeout"), m_adminTimeout);
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
	connect(m_adminLayout, &QComboBox::currentIndexChanged, this, [this] {
		if (m_loading)
			return;
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("admin_keys_layout"), m_adminLayout->currentData().toString() } });
	});
	connect(m_adminTimeout, &QSpinBox::valueChanged, this, [this](int mins) {
		if (m_loading)
			return;
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("admin_keys_timeout_mins"), mins } });
	});
	connect(m_closeTab, &QComboBox::currentIndexChanged, this, [this] {
		if (m_loading)
			return;
		const QString mode = m_closeTab->currentData().toString();
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("close_tab_key"), mode } });
		emit closeTabKeyChanged(mode);
		refreshSummaries();
	});
	connect(m_findKey, &QComboBox::currentIndexChanged, this, [this] {
		if (m_loading)
			return;
		const QString mode = m_findKey->currentData().toString();
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("find_key"), mode } });
		emit findKeyChanged(mode);
	});
	connect(m_tabSwitch, &QComboBox::currentIndexChanged, this, [this] {
		if (!m_loading)
			saveTabSwitchKeys();
	});
	for (QKeySequenceEdit* e : { m_tabPrev, m_tabNext })
	{
		connect(e, &QKeySequenceEdit::editingFinished, this, [this] {
			if (!m_loading)
				saveTabSwitchKeys();
		});
		// Leeren über den Löschknopf meldet kein editingFinished.
		connect(e, &QKeySequenceEdit::keySequenceChanged, this, [this, e](const QKeySequence& k) {
			if (!m_loading && k.isEmpty() && !e->hasFocus())
				saveTabSwitchKeys();
		});
	}
	connect(tabReset, &QPushButton::clicked, this, [this] {
		m_loading = true;
		m_tabPrev->setKeySequence(QKeySequence(QStringLiteral("Ctrl+Shift+PgUp")));
		m_tabNext->setKeySequence(QKeySequence(QStringLiteral("Ctrl+Shift+PgDown")));
		m_loading = false;
		saveTabSwitchKeys();
	});
	return w;
}



QWidget* SettingsTab::pageLicense()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	Q_UNUSED(T);
	buildLicensePage(v, w);
	v->addStretch(1);
	return w;
}

QWidget* SettingsTab::pageMcp()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	// Datenschutz zuerst und deutlich: alles, was Claude liest, geht an den Claude-Dienst.
	auto* privacy = new QLabel(T("settings.mcp.privacy"), w);
	privacy->setWordWrap(true);
	privacy->setProperty("rjWarnText", true);
	v->addWidget(privacy);
	m_mcp = new QCheckBox(T("settings.mcp.enabled"), w);
	v->addWidget(m_mcp);
	m_mcpHideAddr = new QCheckBox(T("settings.mcp.hide.addr"), w);
	m_mcpHideSerial = new QCheckBox(T("settings.mcp.hide.serial"), w);
	v->addWidget(m_mcpHideAddr);
	v->addWidget(m_mcpHideSerial);
	v->addWidget(rjui::section(T("settings.mcp.folders"), w));
	v->addWidget(hint(T("settings.mcp.folders.hint"), w));
	m_mcpFolders = new QListWidget(w);
	m_mcpFolders->setMinimumHeight(160);
	v->addWidget(m_mcpFolders);
	v->addWidget(rjui::section(T("settings.mcp.logins"), w));
	v->addWidget(hint(rjPluginText(m_backend, "settings.mcp.logins.hint"), w));
	m_mcpLogins = new QListWidget(w);
	m_mcpLogins->setMinimumHeight(100);
	v->addWidget(m_mcpLogins);
	v->addWidget(rjui::section(T("settings.mcp.setup"), w));
	m_mcpConfig = hint(QString(), w);
	m_mcpConfig->setTextInteractionFlags(Qt::TextSelectableByMouse);
	v->addWidget(m_mcpConfig);
	auto* copy = new QPushButton(T("settings.mcp.copy"), w);
	auto* bh = new QHBoxLayout;
	bh->addWidget(copy);
	bh->addStretch(1);
	v->addLayout(bh);
	v->addStretch(1);
	for (QCheckBox* b : { m_mcp, m_mcpHideAddr, m_mcpHideSerial })
		connect(b, &QCheckBox::toggled, this, &SettingsTab::saveMcp);
	for (QListWidget* l : { m_mcpFolders, m_mcpLogins })
		connect(l, &QListWidget::itemChanged, this, &SettingsTab::saveMcp);
	connect(copy, &QPushButton::clicked, this, [this] {
		QGuiApplication::clipboard()->setText(m_mcpConfig->property("snippet").toString());
		emit status(rjText(m_backend, "settings.mcp.copied"));
	});
	return w;
}

void SettingsTab::saveMcp()
{
	if (m_loading)
		return;
	QJsonArray folders, logins;
	for (int i = 0; i < m_mcpFolders->count(); ++i)
		if (m_mcpFolders->item(i)->checkState() == Qt::Checked)
			folders.append(m_mcpFolders->item(i)->data(Qt::UserRole).toString());
	for (int i = 0; i < m_mcpLogins->count(); ++i)
		if (m_mcpLogins->item(i)->checkState() == Qt::Checked)
			logins.append(m_mcpLogins->item(i)->data(Qt::UserRole).toString());
	edit({ { QStringLiteral("op"), QStringLiteral("set_mcp_settings") },
	       { QStringLiteral("enabled"), m_mcp->isChecked() },
	       { QStringLiteral("hide_addresses"), m_mcpHideAddr->isChecked() },
	       { QStringLiteral("hide_serials"), m_mcpHideSerial->isChecked() },
	       { QStringLiteral("allowed"), folders },
	       { QStringLiteral("logins"), logins } });
	refreshSummaries();
}

void SettingsTab::reloadMcp()
{
	const bool was = m_loading;
	m_loading = true;
	const QJsonObject s = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("mcp_settings") } }));
	m_mcp->setChecked(s.value(QStringLiteral("enabled")).toBool());
	m_mcpHideAddr->setChecked(s.value(QStringLiteral("hide_addresses")).toBool());
	m_mcpHideSerial->setChecked(s.value(QStringLiteral("hide_serials")).toBool());
	auto fill = [](QListWidget* l, const QJsonArray& all, const QJsonArray& on, const char* label) {
		QStringList checked;
		for (const QJsonValue& x : on)
			checked << x.toString();
		l->clear();
		for (const QJsonValue& x : all)
		{
			const QJsonObject o = x.toObject();
			auto* it = new QListWidgetItem(o.value(QString::fromLatin1(label)).toString(), l);
			it->setData(Qt::UserRole, o.value(QStringLiteral("id")).toString());
			it->setFlags(it->flags() | Qt::ItemIsUserCheckable);
			it->setCheckState(checked.contains(o.value(QStringLiteral("id")).toString()) ? Qt::Checked : Qt::Unchecked);
		}
	};
	fill(m_mcpFolders, s.value(QStringLiteral("folders")).toArray(), s.value(QStringLiteral("allowed")).toArray(), "path");
	fill(m_mcpLogins, s.value(QStringLiteral("all_logins")).toArray(), s.value(QStringLiteral("logins")).toArray(), "name");
	// Eintrag für Claude Desktop (claude_desktop_config.json): rj-mcp liegt neben remotejoe.
#ifdef Q_OS_WIN
	const QString mcpName = QStringLiteral("rj-mcp.exe");
#else
	const QString mcpName = QStringLiteral("rj-mcp");
#endif
	const QString exe = QDir::toNativeSeparators(QDir(QCoreApplication::applicationDirPath()).filePath(mcpName));
	QJsonObject server{ { QStringLiteral("command"), exe } };
	const QString home = qEnvironmentVariable("REMOTEJOE_HOME");
	if (!home.isEmpty())
		server.insert(QStringLiteral("env"), QJsonObject{ { QStringLiteral("REMOTEJOE_HOME"), home } });
	const QString snippet = QString::fromUtf8(QJsonDocument(QJsonObject{ { QStringLiteral("mcpServers"), QJsonObject{ { QStringLiteral("remotejoe"), server } } } }).toJson(QJsonDocument::Indented));
	m_mcpConfig->setProperty("snippet", snippet);
	m_mcpConfig->setText(rjText(m_backend, "settings.mcp.setup.hint") + QStringLiteral("\n\n") + snippet);
	m_loading = was;
}

QWidget* SettingsTab::pageBackup()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	auto* f = new QFormLayout;
	m_bkEnabled = new QCheckBox(T("settings.backup.enabled"), w);
	f->addRow(QString(), m_bkEnabled);
	m_bkLocal = new QLabel(w);
	m_bkLocal->setWordWrap(true);
	m_bkLocal->setTextInteractionFlags(Qt::TextSelectableByMouse);
	f->addRow(T("settings.backup.local"), m_bkLocal);
	auto spin = [w](int lo, int hi, const QString& suffix) {
		auto* s = new QSpinBox(w);
		s->setRange(lo, hi);
		s->setSuffix(suffix);
		return s;
	};
	m_bkLast = spin(1, 100, T("settings.backup.files"));
	f->addRow(T("settings.backup.last"), m_bkLast);
	m_bkDays = spin(0, 90, T("settings.backup.daysuffix"));
	f->addRow(T("settings.backup.days"), m_bkDays);
	m_bkUpdate = spin(1, 20, T("settings.backup.files"));
	f->addRow(T("settings.backup.update"), m_bkUpdate);
	m_bkSecond = new QCheckBox(T("settings.backup.second"), w);
	f->addRow(QString(), m_bkSecond);
	auto* dirRow = new QHBoxLayout;
	m_bkDir = new QLineEdit(w);
	m_bkDir->setPlaceholderText(T("settings.backup.second.ph"));
	auto* browse = new QPushButton(T("settings.backup.browse"), w);
	dirRow->addWidget(m_bkDir, 1);
	dirRow->addWidget(browse);
	f->addRow(T("settings.backup.second.dir"), dirRow);
	f->addRow(QString(), hint(T("settings.backup.hint"), w));
	m_bkKeyFile = hint(T("settings.backup.keyfile"), w);
	f->addRow(QString(), m_bkKeyFile);
	m_bkState = new QLabel(w);
	m_bkState->setWordWrap(true);
	m_bkState->setTextInteractionFlags(Qt::TextSelectableByMouse);
	f->addRow(T("settings.backup.state"), m_bkState);
	auto* buttons = new QHBoxLayout;
	m_bkNow = new QPushButton(T("settings.backup.now"), w);
	auto* openLocal = new QPushButton(T("settings.backup.open"), w);
	auto* openSecond = new QPushButton(T("settings.backup.open2"), w);
	buttons->addWidget(m_bkNow);
	buttons->addWidget(openLocal);
	buttons->addWidget(openSecond);
	buttons->addStretch(1);
	f->addRow(QString(), buttons);
	v->addLayout(f);
	v->addStretch(1);

	connect(m_bkEnabled, &QCheckBox::toggled, this, &SettingsTab::saveBackup);
	connect(m_bkSecond, &QCheckBox::toggled, this, &SettingsTab::saveBackup);
	for (auto* s : { m_bkLast, m_bkDays, m_bkUpdate })
		connect(s, &QSpinBox::valueChanged, this, &SettingsTab::saveBackup);
	connect(m_bkDir, &QLineEdit::editingFinished, this, &SettingsTab::saveBackup);
	connect(browse, &QPushButton::clicked, this, [this] {
		const QString d = QFileDialog::getExistingDirectory(this, rjText(m_backend, "settings.backup.second.dir"), m_bkDir->text());
		if (d.isEmpty())
			return;
		m_bkDir->setText(QDir::toNativeSeparators(d));
		m_bkSecond->setChecked(true);
		saveBackup();
	});
	connect(m_bkNow, &QPushButton::clicked, this, [this] {
		const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("backup_now") } }));
		if (r.contains(QStringLiteral("error")))
		{
			emit status(r.value(QStringLiteral("error")).toString());
			return;
		}
		m_bkNow->setEnabled(false);
		m_bkState->setText(rjText(m_backend, "settings.backup.busy"));
	});
	connect(openLocal, &QPushButton::clicked, this, [this] {
		if (!m_bkLocalDir.isEmpty() && QDir().mkpath(m_bkLocalDir))
			QDesktopServices::openUrl(QUrl::fromLocalFile(m_bkLocalDir));
	});
	connect(openSecond, &QPushButton::clicked, this, [this] {
		if (!m_bkDir->text().trimmed().isEmpty())
			QDesktopServices::openUrl(QUrl::fromLocalFile(m_bkDir->text().trimmed()));
	});
	return w;
}

QWidget* SettingsTab::pageWebCache()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	auto* f = new QFormLayout;
	m_wcLimit = new QSpinBox(w);
	m_wcLimit->setRange(10, 2000);
	m_wcLimit->setSingleStep(10);
	m_wcLimit->setSuffix(T("settings.webcache.mb"));
	f->addRow(T("settings.webcache.limit"), m_wcLimit);
	m_wcDays = new QSpinBox(w);
	m_wcDays->setRange(1, 365);
	m_wcDays->setSuffix(T("settings.webcache.days.suffix"));
	f->addRow(T("settings.webcache.days"), m_wcDays);
	f->addRow(QString(), hint(T("settings.webcache.hint"), w));
	m_wcState = new QLabel(w);
	m_wcState->setWordWrap(true);
	m_wcState->setTextInteractionFlags(Qt::TextSelectableByMouse);
	f->addRow(T("settings.webcache.state"), m_wcState);
	auto* buttons = new QHBoxLayout;
	auto* now = new QPushButton(T("settings.webcache.now"), w);
	now->setObjectName(QStringLiteral("webCacheNow"));
	auto* open = new QPushButton(T("settings.webcache.open"), w);
	buttons->addWidget(now);
	buttons->addWidget(open);
	buttons->addStretch(1);
	f->addRow(QString(), buttons);
	m_wcResult = new QLabel(w);
	m_wcResult->setWordWrap(true);
	f->addRow(QString(), m_wcResult);
	v->addLayout(f);
	v->addStretch(1);
	for (auto* s : { m_wcLimit, m_wcDays })
		connect(s, &QSpinBox::valueChanged, this, &SettingsTab::saveWebCache);
	connect(now, &QPushButton::clicked, this, [this] {
		const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("webcache_cleanup") }, { QStringLiteral("all"), true } }));
		if (r.contains(QStringLiteral("error")))
		{
			m_wcResult->setText(r.value(QStringLiteral("error")).toString());
			return;
		}
		QString text = rjText(m_backend, "settings.webcache.done")
		                 .arg(QLocale().formattedDataSize(static_cast<qint64>(r.value(QStringLiteral("freed")).toDouble()), 2, QLocale::DataSizeTraditionalFormat))
		                 .arg(r.value(QStringLiteral("cleared")).toInt())
		                 .arg(r.value(QStringLiteral("removed")).toInt());
		if (r.value(QStringLiteral("locked")).toBool())
			text += QLatin1Char(' ') + rjText(m_backend, "settings.webcache.locked");
		m_wcResult->setText(text);
		reloadWebCache();
	});
	connect(open, &QPushButton::clicked, this, [this] {
		const QString dir = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("webcache_status") } })).value(QStringLiteral("dir")).toString();
		if (!dir.isEmpty() && QDir().mkpath(dir))
			QDesktopServices::openUrl(QUrl::fromLocalFile(dir));
	});
	return w;
}

QWidget* SettingsTab::pageDiag()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	auto* f = new QFormLayout;
	m_diag = new QCheckBox(T("settings.diag.on"), w);
	m_diag->setObjectName(QStringLiteral("diagSwitch"));
	f->addRow(QString(), m_diag);
	f->addRow(QString(), hint(T("settings.diag.hint"), w));
	m_diagEnv = hint(T("settings.diag.env"), w);
	f->addRow(QString(), m_diagEnv);
	m_diagDir = new QLabel(w);
	m_diagDir->setWordWrap(true);
	m_diagDir->setTextInteractionFlags(Qt::TextSelectableByMouse);
	f->addRow(T("settings.diag.dir"), m_diagDir);
	auto* buttons = new QHBoxLayout;
	auto* open = new QPushButton(T("settings.diag.open"), w);
	buttons->addWidget(open);
	buttons->addStretch(1);
	f->addRow(QString(), buttons);
	v->addLayout(f);
	v->addStretch(1);
	connect(m_diag, &QCheckBox::toggled, this, [this](bool on) {
		if (m_loading)
			return;
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("diag"), on } });
		refreshSummaries();
	});
	connect(open, &QPushButton::clicked, this, [this] {
		const QString d = m_diagDir->text();
		if (!d.isEmpty() && QDir().mkpath(d))
			QDesktopServices::openUrl(QUrl::fromLocalFile(d));
	});
	return w;
}

void SettingsTab::saveWebCache()
{
	if (m_loading)
		return;
	edit({ { QStringLiteral("op"), QStringLiteral("set_settings") },
	       { QStringLiteral("webcache_limit_mb"), m_wcLimit->value() },
	       { QStringLiteral("webcache_keep_days"), m_wcDays->value() } });
	refreshSummaries();
}

void SettingsTab::reloadWebCache()
{
	const bool was = m_loading;
	m_loading = true;
	const QJsonObject s = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("webcache_status") } }));
	m_wcLimit->setValue(s.value(QStringLiteral("limit_mb")).toInt(50));
	m_wcDays->setValue(s.value(QStringLiteral("keep_days")).toInt(7));
	m_wcState->setText(rjText(m_backend, "settings.webcache.size")
	                     .arg(QLocale().formattedDataSize(static_cast<qint64>(s.value(QStringLiteral("bytes")).toDouble()), 2, QLocale::DataSizeTraditionalFormat))
	                     .arg(s.value(QStringLiteral("profiles")).toInt())
	                     .arg(QLocale().formattedDataSize(static_cast<qint64>(s.value(QStringLiteral("cache_bytes")).toDouble()), 2, QLocale::DataSizeTraditionalFormat)));
	m_loading = was;
}

void SettingsTab::saveBackup()
{
	if (m_loading)
		return;
	m_bkDir->setEnabled(m_bkSecond->isChecked());
	edit({ { QStringLiteral("op"), QStringLiteral("set_settings") },
	       { QStringLiteral("backup_enabled"), m_bkEnabled->isChecked() },
	       { QStringLiteral("backup_keep_last"), m_bkLast->value() },
	       { QStringLiteral("backup_keep_days"), m_bkDays->value() },
	       { QStringLiteral("backup_keep_update"), m_bkUpdate->value() },
	       { QStringLiteral("backup_second_enabled"), m_bkSecond->isChecked() },
	       { QStringLiteral("backup_second_dir"), m_bkDir->text().trimmed() } });
	refreshSummaries();
}

void SettingsTab::reloadBackup()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	const bool was = m_loading;
	m_loading = true;
	const QJsonObject b = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("backup_status") } }));
	m_bkEnabled->setChecked(b.value(QStringLiteral("enabled")).toBool(true));
	m_bkLast->setValue(b.value(QStringLiteral("keep_last")).toInt(10));
	m_bkDays->setValue(b.value(QStringLiteral("keep_days")).toInt(7));
	m_bkUpdate->setValue(b.value(QStringLiteral("keep_update")).toInt(3));
	m_bkSecond->setChecked(b.value(QStringLiteral("second_enabled")).toBool());
	m_bkDir->setText(b.value(QStringLiteral("second_dir")).toString());
	m_bkDir->setEnabled(m_bkSecond->isChecked());
	m_bkLocalDir = b.value(QStringLiteral("local_dir")).toString();
	m_bkLocal->setText(m_bkLocalDir.isEmpty() ? T("settings.backup.nokdbx") : QDir::toNativeSeparators(m_bkLocalDir));
	m_bkKeyFile->setVisible(b.value(QStringLiteral("key_file")).toBool());
	const bool busy = b.value(QStringLiteral("busy")).toBool();
	m_bkNow->setEnabled(!busy);
	const QJsonObject last = b.value(QStringLiteral("last")).toObject();
	if (busy)
		m_bkState->setText(T("settings.backup.busy"));
	else if (last.isEmpty())
		m_bkState->setText(T("settings.backup.never"));
	else
	{
		QStringList lines;
		for (const auto& t : last.value(QStringLiteral("targets")).toArray())
		{
			const QJsonObject o = t.toObject();
			const QString dir = QDir::toNativeSeparators(o.value(QStringLiteral("dir")).toString());
			lines << (o.value(QStringLiteral("ok")).toBool() ? T("settings.backup.target.ok").arg(dir)
			                                                  : T("settings.backup.target.err").arg(dir, o.value(QStringLiteral("error")).toString()));
		}
		m_bkState->setText(T("settings.backup.at").arg(last.value(QStringLiteral("at")).toString()) + QLatin1Char('\n') + lines.join(QLatin1Char('\n')));
	}
	m_loading = was;
	refreshSummaries();
}

// Cipher-Suites: ältere SSH-Verfahren für Altgeräte, in zwei Stufen, Vorgabe aus; mit Warnung.
QWidget* SettingsTab::pageCiphers()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	auto* warn = rjui::pill(T("settings.ciphers.warn"), QStringLiteral("warn"), w);
	warn->setWordWrap(true);
	v->addWidget(warn);
	auto* f = new QFormLayout;
	m_cipherOld = new QCheckBox(T("settings.ciphers.old"), w);
	f->addRow(QString(), m_cipherOld);
	f->addRow(QString(), hint(T("settings.ciphers.old.hint"), w));
	m_cipherVeryOld = new QCheckBox(T("settings.ciphers.veryold"), w);
	f->addRow(QString(), m_cipherVeryOld);
	f->addRow(QString(), hint(T("settings.ciphers.veryold.hint"), w));
	f->addRow(QString(), hint(T("settings.ciphers.target.hint"), w));
	v->addLayout(f);
	v->addStretch(1);
	auto save = [this] {
		if (m_loading)
			return;
		// Stufe 2 nur zusammen mit Stufe 1.
		if (!m_cipherOld->isChecked() && m_cipherVeryOld->isChecked())
		{
			const bool was = m_loading;
			m_loading = true;
			m_cipherVeryOld->setChecked(false);
			m_loading = was;
		}
		m_cipherVeryOld->setEnabled(m_cipherOld->isChecked());
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("ssh_legacy_ciphers"), m_cipherOld->isChecked() },
		       { QStringLiteral("ssh_very_old_ciphers"), m_cipherVeryOld->isChecked() } });
		refreshSummaries();
	};
	connect(m_cipherOld, &QCheckBox::toggled, this, save);
	connect(m_cipherVeryOld, &QCheckBox::toggled, this, save);
	return w;
}

void SettingsTab::reload()
{
	m_loading = true;
	const QJsonObject s = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("settings") } }));
	if (const int i = m_idle->findData(s.value(QStringLiteral("lock_after_idle_mins")).toInt()); i >= 0)
		m_idle->setCurrentIndex(i);
	m_disconnect->setChecked(s.value(QStringLiteral("disconnect_on_lock")).toBool());
	m_keyErrors->setChecked(s.value(QStringLiteral("accept_key_errors")).toBool());
	m_diag->setChecked(s.value(QStringLiteral("diag")).toBool() || s.value(QStringLiteral("diag_env")).toBool());
	m_diag->setEnabled(!s.value(QStringLiteral("diag_env")).toBool());
	m_diagEnv->setVisible(s.value(QStringLiteral("diag_env")).toBool());
	m_diagDir->setText(QDir::toNativeSeparators(s.value(QStringLiteral("diag_dir")).toString()));
	m_cipherOld->setChecked(s.value(QStringLiteral("ssh_legacy_ciphers")).toBool());
	m_cipherVeryOld->setChecked(s.value(QStringLiteral("ssh_very_old_ciphers")).toBool());
	m_cipherVeryOld->setEnabled(m_cipherOld->isChecked());
	m_clipSecs->setValue(m_backend->clipboardClearSecs());
	m_keyCapture->setChecked(FrameView::s_keyCapture);
	if (const int i = m_closeTab->findData(s.value(QStringLiteral("close_tab_key")).toString(QStringLiteral("outside"))); i >= 0)
		m_closeTab->setCurrentIndex(i);
	if (const int i = m_findKey->findData(s.value(QStringLiteral("find_key")).toString(QStringLiteral("outside"))); i >= 0)
		m_findKey->setCurrentIndex(i);
	if (const int i = m_tabSwitch->findData(s.value(QStringLiteral("tab_switch_key")).toString(QStringLiteral("outside"))); i >= 0)
		m_tabSwitch->setCurrentIndex(i);
	m_tabPrev->setKeySequence(QKeySequence(s.value(QStringLiteral("tab_prev_keys")).toString(QStringLiteral("Ctrl+Shift+PgUp")), QKeySequence::PortableText));
	m_tabNext->setKeySequence(QKeySequence(s.value(QStringLiteral("tab_next_keys")).toString(QStringLiteral("Ctrl+Shift+PgDown")), QKeySequence::PortableText));
	m_adminLayout->clear();
	for (const QJsonValue& l : s.value(QStringLiteral("admin_keys_layouts")).toArray())
	{
		const QJsonObject o = l.toObject();
		m_adminLayout->addItem(o.value(QStringLiteral("name")).toString(), o.value(QStringLiteral("klid")).toString());
		// Belegungen ohne Tabelle: sichtbar, aber nicht wählbar.
		if (!o.value(QStringLiteral("available")).toBool())
			if (auto* model = qobject_cast<QStandardItemModel*>(m_adminLayout->model()))
				model->item(m_adminLayout->count() - 1)->setEnabled(false);
	}
	if (const int i = m_adminLayout->findData(s.value(QStringLiteral("admin_keys_layout")).toString()); i >= 0)
		m_adminLayout->setCurrentIndex(i);
	m_adminTimeout->setValue(s.value(QStringLiteral("admin_keys_timeout_mins")).toInt(10));
	for (RjSettingsPart* part : m_parts)
		part->load(s);
	reloadBackup();
	reloadWebCache();
	reloadMcp();
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
			it->item->setToolTip(0, text);
	};
	const int mins = m_idle->currentData().toInt();
	const int secs = m_clipSecs->value();
	set("lock", T("settings.lock.sum")
	              .arg(mins == 0 ? T("idle.off") : T("idle.minutes").arg(mins))
	              .arg(secs == 0 ? T("settings.clip.never") : QString::number(secs) + T("settings.clip.suffix"))
	            + (m_keyErrors->isChecked() ? T("settings.keyerrors.sum") : QString()));
	set("keys", T(m_keyCapture->isChecked() ? "settings.keys.sum.on" : "settings.keys.sum.off"));
	set("ciphers", T("settings.ciphers.sum").arg(T(!m_cipherOld->isChecked() ? "settings.ciphers.sum.off" : m_cipherVeryOld->isChecked() ? "settings.ciphers.sum.veryold" : "settings.ciphers.sum.old")));
	set("terminal", T("settings.terminal.sum"));
	set("appearance", T("settings.appearance.sum"));
	set("backup", m_bkEnabled->isChecked() ? T("settings.backup.sum.on").arg(m_bkLast->value()).arg(m_bkDays->value())
	                                       + (m_bkSecond->isChecked() && !m_bkDir->text().trimmed().isEmpty() ? T("settings.backup.sum.second") : QString())
	                                       : T("settings.backup.sum.off"));
	set("webcache", T("settings.webcache.sum").arg(m_wcLimit->value()).arg(m_wcDays->value()));
	set("diag", T(m_diag->isChecked() ? "settings.diag.sum.on" : "settings.diag.sum.off"));
	if (m_mcp)
		set("mcp", T(m_mcp->isChecked() ? "settings.mcp.sum.on" : "settings.mcp.sum.off"));
	for (RjSettingsPart* part : m_parts)
		part->summaries([&set](const char* key, const QString& text) { set(key, text); });
	set("license", licenseSummary());
}

void SettingsTab::wipeSecrets()
{
	if (m_snmp)
		m_snmp->wipeSecrets();
}
