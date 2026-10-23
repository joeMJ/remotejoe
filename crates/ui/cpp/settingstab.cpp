#include "settingstab.h"

#include "appearance.h"
#include "cards.h"
#include "kbcapture.h"
#include "views.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDir>
#include <QtCore/QJsonArray>
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
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QListWidget>
#include <QtGui/QClipboard>
#include <QtGui/QGuiApplication>
#include <QtCore/QCoreApplication>
#include <QtWidgets/QPushButton>
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
	addArea(QStringLiteral("backup"), T("settings.backup"), pageBackup(), true);
	addArea(QStringLiteral("features"), T("settings.features"), pageFeatures(), true);
	addArea(QStringLiteral("mcp"), T("settings.mcp"), pageMcp(), true);
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

QWidget* SettingsTab::pageFeatures()
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	auto* f = new QFormLayout;
	m_aruba = new QCheckBox(T("settings.features.aruba"), w);
	f->addRow(QString(), m_aruba);
	f->addRow(QString(), hint(T("settings.features.aruba.hint"), w));
	// Anwender je AP: Farbe der Zahl auf der Kachel (0 = aus).
	auto clients = [&](const char* label) {
		auto* s = new QSpinBox(w);
		s->setRange(0, 1000);
		s->setSpecialValueText(T("settings.aruba.clients.off"));
		s->setSuffix(T("settings.aruba.clients.suffix"));
		f->addRow(T(label), s);
		return s;
	};
	m_clientsWarn = clients("settings.aruba.clients.warn");
	m_clientsCrit = clients("settings.aruba.clients.crit");
	m_clientsHint = hint(T("settings.aruba.clients.hint"), w);
	f->addRow(QString(), m_clientsHint);
	v->addLayout(f);
	v->addStretch(1);
	auto saveClients = [this] {
		const int warn = m_clientsWarn->value(), crit = m_clientsCrit->value();
		const bool bad = warn > 0 && crit > 0 && crit <= warn;
		m_clientsHint->setText(rjText(m_backend, bad ? "settings.aruba.clients.bad" : "settings.aruba.clients.hint"));
		if (m_loading)
			return;
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("aruba_clients_warn"), warn }, { QStringLiteral("aruba_clients_crit"), crit } });
	};
	connect(m_clientsWarn, &QSpinBox::valueChanged, this, saveClients);
	connect(m_clientsCrit, &QSpinBox::valueChanged, this, saveClients);
	connect(m_aruba, &QCheckBox::toggled, this, [this](bool on) {
		if (m_loading)
			return;
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("aruba_tools"), on } });
		emit featuresChanged();
		refreshSummaries();
	});
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
	v->addWidget(hint(T("settings.mcp.logins.hint"), w));
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
	m_aruba->setChecked(s.value(QStringLiteral("aruba_tools")).toBool());
	m_clientsWarn->setValue(s.value(QStringLiteral("aruba_clients_warn")).toInt(30));
	m_clientsCrit->setValue(s.value(QStringLiteral("aruba_clients_crit")).toInt(64));
	reloadBackup();
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
	set("backup", m_bkEnabled->isChecked() ? T("settings.backup.sum.on").arg(m_bkLast->value()).arg(m_bkDays->value())
	                                       + (m_bkSecond->isChecked() && !m_bkDir->text().trimmed().isEmpty() ? T("settings.backup.sum.second") : QString())
	                                       : T("settings.backup.sum.off"));
	if (m_mcp)
		set("mcp", T(m_mcp->isChecked() ? "settings.mcp.sum.on" : "settings.mcp.sum.off"));
	set("features", T(m_aruba->isChecked() ? "settings.features.sum.aruba" : "settings.features.sum.none"));
}

void SettingsTab::wipeSecrets()
{
	if (m_snmp)
		m_snmp->wipeSecrets();
}
