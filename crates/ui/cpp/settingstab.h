#pragma once

#include <QtCore/QHash>

// Extras → Einstellungen: ein Reiter im Kartenstil wie die Einstellungen eines Ziels. Kopfkarte, links die
// Bereiche (Sperre und Zwischenablage, Tastatur, Terminal, Darstellung, SNMP-Geräte), rechts die Karte des
// gewählten Bereichs. Alle Änderungen gelten sofort und werden gespeichert; es gibt nichts zu speichern oder
// zu verwerfen.

#include "editor.h"
#include "rjpluginui.h"

#include <vector>

#include <QtCore/QMap>
#include <QtCore/QString>
#include <functional>

class AppearanceTab;
class QCheckBox;
class QComboBox;
class QKeySequenceEdit;
class QLabel;
class QLineEdit;
class QListWidget;
class QPushButton;
class QSpinBox;
class QStackedWidget;
class QVBoxLayout;
class RjBackend;
class QTreeWidgetItem;
class SnmpTab;
namespace rjui
{
class ClickRow;
}

class SettingsTab : public ToolTab, public RjHostSettings
{
	Q_OBJECT
public:
	explicit SettingsTab(RjBackend* backend, QWidget* parent = nullptr);
	/// Bereich zeigen: lock, keys, terminal, appearance, snmp.
	void showSection(const QString& key);
	QString currentSection() const { return m_current; }
	void wipeSecrets() override;
	/// Werte neu lesen (nach Entsperren oder Änderungen von außen).
	void reload();
	/// Stand der Sicherung neu zeigen (nach einer Sicherung im Hintergrund).
	void reloadBackup();
	void reloadWebCache();
	/// Lizenzseite neu aufbauen (nach Lizenzeingabe, Entfernen, Downloads).
	void rebuildLicensePage();
	/// Ergebnis eines Downloads einer Erweiterung (Art `plugin_fetch`): Zeile der Erweiterung aktualisieren.
	void licenseResult(const QJsonObject& r);
	/// Prüfdialog „Erweiterungen aus Datei einspielen“ zum Prüfergebnis `r` (op plugins_file_check) der Dateien
	/// `files`; „Einspielen“ nur, wenn alles passt. Nicht gezeigt (show/exec beim Aufrufer).
	class QDialog* fileCheckDialog(const QJsonObject& r, const QStringList& files);

	// Host-Schnittstelle für Erweiterungen (plugin/rjpluginui.h): Bereiche, Gruppen, Titel, Bearbeiten über den Kern.
	RjBackend* backend() const { return m_backend; }
	RjHostBackend* hostBackend() const override;
	QWidget* widget() override { return this; }
	/// Statuszeile (ToolTab::status als Signal).
	void status(const QString& text) override { emit ToolTab::status(text); }
	void addArea(const QString& key, const QString& title, QWidget* page, bool scroll, int indent = 0) override;
	/// Überschrift ohne eigene Seite (z. B. eine Gruppe), eingerückt um indent Stufen.
	void addGroup(const QString& title, int indent) override;
	void setAreaTitle(const QString& key, const QString& title) override { m_titles.insert(key, title); }
	QString edit(const QJsonObject& cmd) override;
	void refreshSummaries() override;
	/// Werte werden gerade geladen (Änderungen nicht speichern).
	bool loading() const override { return m_loading; }

signals:
	/// Farbschema oder Fokusrahmen geändert (Oberfläche neu einfärben).
	void appearanceChanged();
	/// Strg+Umschalt+W: neuer Modus (outside, always, never).
	void closeTabKeyChanged(const QString& mode);
	void findKeyChanged(const QString& mode);
	/// Tab-Wechsel: Modus (outside, always, never) und Tastenfolgen nach links/rechts.
	void tabSwitchKeysChanged(const QString& mode, const QString& prev, const QString& next);
	/// Zuschaltbare Funktionen ein- oder ausgeschaltet.
	void featuresChanged() override;
	/// Baum geändert (z. B. Vorlagenordner angelegt).
	void treeChanged() override;

private:
	struct Area
	{
		QTreeWidgetItem* item;
		int page;
	};
	/// Gruppe mit Symbol (Kern); `addGroup` der Erweiterungen nimmt ein allgemeines Symbol.
	void addGroupIcon(const QString& glyph, const QString& title, int indent);
	QHash<int, QTreeWidgetItem*> m_groupAt;
	rjui::AreaTree* m_areaTree = nullptr;
	QWidget* pageLock();
	QWidget* pageKeys();
	QWidget* pageCiphers();
	QWidget* pageLicense();
	/// Lizenz: Urkunde, Erweiterungen, Update-Quelle, Einfügen (licensepage.cpp).
	void buildLicensePage(QVBoxLayout* v, QWidget* w);
	void fillLicensePage();
	QString pluginStateText(const QString& id, const QJsonObject& j, const QJsonObject& mod) const;
	QString licenseSummary() const;
	QWidget* pageBackup();
	QWidget* pageWebCache();
	QWidget* pageMcp();
	void saveMcp();
	void reloadMcp();
	void saveBackup();
	void saveWebCache();

	RjBackend* m_backend;
	QStackedWidget* m_pages = nullptr;
	QLabel* m_pageTitle = nullptr;
	QMap<QString, Area> m_areas;
	QMap<QString, QString> m_titles;
	QString m_current;
	bool m_loading = false;
	// Lizenzseite (veränderlicher Teil)
	QWidget* m_licenseBox = nullptr;
	/// Updates → Erweiterungen: neu aufgebauter Teil (Bezug, Kennwort, Stand) und bleibendes Protokoll.
	QWidget* m_extBox = nullptr;
	class QPlainTextEdit* m_extLog = nullptr;
	QHash<QString, QLabel*> m_extState;
	QWidget* pageExtensions();
	/// Erweiterungen-Seite füllen (aus fillLicensePage); `l`: Lizenzangaben, `j`: ganzes Lizenz-JSON.
	void fillExtensionsPage(const QJsonObject& j, const QJsonObject& l, bool valid);
	void extLog(const QString& text);
	/// „Aus Datei einspielen …“: Dateien wählen, prüfen, Prüfdialog, einspielen.
	void pluginsFromFile();
	QWidget* m_licenseRestart = nullptr;
	QPushButton* m_licenseRemoveBtn = nullptr;
	bool m_licenseRebuildPending = false;
	QMap<QString, QLabel*> m_pluginState;
	QMap<QString, QJsonObject> m_pluginRows;
	QMap<QString, QJsonObject> m_pluginLast;
	QString m_pluginSource;
	bool m_licenseApplied = false;
	bool m_licenseRemoved = false;
	// Sperre und Zwischenablage
	QComboBox* m_idle = nullptr;
	QCheckBox* m_disconnect = nullptr;
	QCheckBox* m_keyErrors = nullptr;
	QCheckBox* m_cipherOld = nullptr;
	QCheckBox* m_cipherVeryOld = nullptr;
	QSpinBox* m_clipSecs = nullptr;
	// Tastatur
	QCheckBox* m_keyCapture = nullptr;
	QComboBox* m_closeTab = nullptr;
	QComboBox* m_findKey = nullptr;
	QComboBox* m_tabSwitch = nullptr;
	QKeySequenceEdit* m_tabPrev = nullptr;
	QKeySequenceEdit* m_tabNext = nullptr;
	void saveTabSwitchKeys();
	QComboBox* m_adminLayout = nullptr;
	QSpinBox* m_adminTimeout = nullptr;
	// Claude-Anbindung
	QCheckBox* m_mcp = nullptr;
	QCheckBox* m_mcpHideAddr = nullptr;
	QCheckBox* m_mcpHideSerial = nullptr;
	QListWidget* m_mcpFolders = nullptr;
	QListWidget* m_mcpLogins = nullptr;
	QLabel* m_mcpConfig = nullptr;
	// Sicherung
	QCheckBox* m_bkEnabled = nullptr;
	QSpinBox* m_bkLast = nullptr;
	QSpinBox* m_bkDays = nullptr;
	QSpinBox* m_bkUpdate = nullptr;
	QCheckBox* m_bkSecond = nullptr;
	QLineEdit* m_bkDir = nullptr;
	QLabel* m_bkLocal = nullptr;
	QLabel* m_bkState = nullptr;
	QLabel* m_bkKeyFile = nullptr;
	QPushButton* m_bkNow = nullptr;
	QString m_bkLocalDir;
	QSpinBox* m_wcLimit = nullptr;
	QSpinBox* m_wcDays = nullptr;
	QLabel* m_wcState = nullptr;
	QLabel* m_wcResult = nullptr;
	/// Beiträge der Erweiterungen (Bereiche unter „Erweiterungen“).
	std::vector<class RjSettingsPart*> m_parts;
	// Eingebettete Bereiche
	AppearanceTab* m_appearance = nullptr;
	SnmpTab* m_snmp = nullptr;
};
