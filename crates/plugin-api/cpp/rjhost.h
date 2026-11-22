// Host-Schnittstelle des Kerns für die Oberfläche einer Erweiterung: nur rein virtuelle Klassen und Qt-Typen, keine
// Kopfdateien des Kerns. Die Erweiterung wird als Bibliothek geladen; alles, was sie vom Kern braucht, bekommt sie
// über diese Zeiger (Fassung: RJ_HOST_UI_VERSION, geprüft beim Laden).
#pragma once

#include <QtCore/QJsonObject>
#include <QtCore/QModelIndex>
#include <QtCore/QPoint>
#include <QtCore/QString>
#include <QtCore/QStringList>
#include <QtGui/QIcon>

#include <functional>
#include <vector>

#include <QtGui/QStandardItemModel>
#include <QtWidgets/QMenu>
#include <QtWidgets/QStatusBar>
#include <QtWidgets/QTabWidget>
#include <QtWidgets/QTreeView>
#include <QtWidgets/QWidget>

class RjHostOverview;

/// Fassung dieser Oberflächen-Schnittstelle; Kern und Erweiterung müssen übereinstimmen.
inline constexpr int RJ_HOST_UI_VERSION = 2;

/// Rollen der Baumeinträge (QStandardItemModel des Kerns).
namespace rjrole {
inline constexpr int Id = Qt::UserRole + 1;
inline constexpr int Folder = Qt::UserRole + 2;
inline constexpr int Guest = Qt::UserRole + 3;
inline constexpr int GuestTarget = Qt::UserRole + 4;
inline constexpr int ConsoleHost = Qt::UserRole + 5;
inline constexpr int Placeholder = Qt::UserRole + 6;
inline constexpr int Protocol = Qt::UserRole + 7;
inline constexpr int Light = Qt::UserRole + 8;
inline constexpr int SessionMark = Qt::UserRole + 9;
inline constexpr int Login = Qt::UserRole + 10;
inline constexpr int GuestState = Qt::UserRole + 11;
inline constexpr int IconKind = Qt::UserRole + 12;
inline constexpr int Macro = Qt::UserRole + 20;
inline constexpr int OtherWay = Qt::UserRole + 40;
/// Ordner einer Erweiterung (eigenes Symbol, Einfachklick öffnet ihre Karte).
inline constexpr int Site = Qt::UserRole + 41;
/// Virtueller Switch (Name; kein Ziel).
inline constexpr int VSwitch = Qt::UserRole + 42;
} // namespace rjrole

/// Der Kern (Rust) aus Sicht der Erweiterung: Texte, Bearbeitungsbefehle als JSON, Freischaltung, Symbole.
class RjHostBackend
{
public:
	virtual ~RjHostBackend() = default;
	/// Text zu einem Schlüssel (Kern und Erweiterungen).
	virtual QString text(const QString& key) const = 0;
	/// Bearbeitungsbefehl des Kerns: JSON hinein, JSON heraus (`{"error": …}` bei Fehlern).
	virtual QString edit(const QString& cmdJson) = 0;
	virtual bool moduleActive(const QString& id) const = 0;
	virtual bool modulesActive() const = 0;
	/// Symbol wie im Baum (`kind`: Protokoll oder Geräteart, `light`: Ampel, `session`: Sitzungspunkt 0–2).
	virtual QIcon icon(const QString& kind, const QString& light, bool folder, int session) const = 0;
};

/// Beiträge der Erweiterung zu Menüs und Klicks des Hauptfensters; angemeldet in RjPluginUi::attach().
struct RjModuleHooks
{
	/// Einfachklick vor allem anderen (z. B. virtueller Switch); `true` = erledigt.
	std::vector<std::function<bool(const QModelIndex&)>> clickTop;
	/// Einfachklick auf einen Ordner (einer Erweiterung); `true` = erledigt, sonst Ordnerübersicht.
	std::vector<std::function<bool(const QModelIndex&)>> clickFolder;
	/// Kontextmenü eines Baumeintrags, nach „Löschen“ (Eintrag, Kennung, Ordner).
	std::vector<std::function<void(QMenu&, const QModelIndex&, const QString&, bool)>> treeMenu;
	/// Kontextmenü der Auswahl, nach „Inventar mehrerer Ziele“.
	std::vector<std::function<void(QMenu&)>> selectionMenu;
	/// Hinzufügen-Menü, ganz oben (übergeordneter Ordner).
	std::vector<std::function<void(QMenu*, const QString&)>> addMenu;
	/// Menü Extras, an der bisherigen Stelle.
	std::vector<std::function<void(QMenu*)>> extrasMenu;
	/// Ergebnis eines Hintergrundauftrags (Art, Ziel, Ergebnis); `true` = erledigt.
	std::vector<std::function<bool(const QString&, const QString&, const QJsonObject&)>> jobResult;
	/// Eintrag öffnen (Eingabetaste/Doppelklick), Kontextmenü, Umbenennen, Löschen; `true` = erledigt.
	std::vector<std::function<bool(const QModelIndex&)>> activate;
	std::vector<std::function<bool(const QModelIndex&, const QPoint&)>> contextMenu;
	std::vector<std::function<bool(const QModelIndex&)>> rename;
	std::vector<std::function<bool(const QModelIndex&)>> remove;
	/// Freigegebene Aktion der Claude-Anbindung (Aktion, Ziel-ID); `true` = erledigt.
	std::vector<std::function<bool(const QString&, const QString&)>> mcpAction;
	/// Bildschirmfotos der Vorführung (warten, Bild speichern unter Name).
	std::vector<std::function<void(const std::function<void(int)>&, const std::function<void(const QString&)>&)>> screenshots;
};

/// Das Hauptfenster aus Sicht der Erweiterung.
class RjHostWindow
{
public:
	virtual ~RjHostWindow() = default;
	virtual RjHostBackend* hostBackend() const = 0;
	/// Das Fenster selbst (Elternobjekt für Dialoge, findChildren).
	virtual QWidget* widget() = 0;
	virtual QTabWidget* tabs() const = 0;
	virtual QTreeView* tree() const = 0;
	virtual QStandardItemModel* model() const = 0;
	virtual QStatusBar* statusBar() const = 0;
	/// Tab einer Aufgabe öffnen. Signale des Tabs, die der Kern verbindet, wenn vorhanden: `indexChanged()`,
	/// `status(QString)`, `closeRequested()`; Methode `wipeSecrets()` beim Schließen, wenn vorhanden.
	virtual void addToolTab(QWidget* tab, const QString& title) = 0;
	virtual void closeToolTab(QWidget* tab) = 0;
	virtual QString edit(const QJsonObject& cmd) = 0;
	virtual QString currentId() const = 0;
	virtual QStringList selectedIds() const = 0;
	virtual void selectId(const QString& id) = 0;
	virtual void reloadTree() = 0;
	virtual void renameSelected() = 0;
	virtual void deleteSelected() = 0;
	virtual void showEditResult(const QJsonObject& r, const QString& okMessage = QString()) = 0;
	virtual void openOverview(const QJsonObject& subject, const QIcon& icon = QIcon()) = 0;
	virtual void showProperties(const QString& id, const QString& category = QString()) = 0;
	/// Verbindung über einen Zugangsweg (`way` = „main“ oder die ID des Wegs).
	virtual void connectWay(const QJsonObject& subject, const QString& way) = 0;
	/// Erste offene Objektübersicht (null, wenn keine).
	virtual RjHostOverview* findOverview() = 0;
	/// Leiste im Fenster: Frage mit Knöpfen (Rückruf mit Nummer des Knopfs, -1 = abgebrochen) bzw. kurze Eingabe.
	virtual void ask(const QString& text, const QStringList& buttons, const std::function<void(int)>& done) = 0;
	virtual void prompt(const QString& text, const QString& preset, const std::function<void(const QString&)>& done) = 0;
	virtual RjModuleHooks& hooks() = 0;
	/// Oberfläche einer anderen geladenen Erweiterung (Kennung), sonst nullptr – für Anschlussstellen zwischen
	/// Erweiterungen (RjPluginUi::interface).
	virtual class RjPluginUi* pluginUi(const QString& id) = 0;
};

/// Die Objektübersicht eines Ziels aus Sicht der Erweiterung.
class RjHostOverview
{
public:
	virtual ~RjHostOverview() = default;
	virtual RjHostBackend* hostBackend() const = 0;
	virtual const QJsonObject& subject() const = 0;
	virtual QJsonObject call(const QJsonObject& cmd) const = 0;
	virtual QWidget* widget() = 0;
	virtual QString targetName() const = 0;
	virtual void statusMessage(const QString& text) = 0;
};

/// Die Einstellungen aus Sicht der Erweiterung.
class RjHostSettings
{
public:
	virtual ~RjHostSettings() = default;
	virtual RjHostBackend* hostBackend() const = 0;
	virtual QWidget* widget() = 0;
	virtual void addArea(const QString& key, const QString& title, QWidget* page, bool scroll, int indent = 0) = 0;
	virtual void addGroup(const QString& title, int indent) = 0;
	virtual void setAreaTitle(const QString& key, const QString& title) = 0;
	virtual QString edit(const QJsonObject& cmd) = 0;
	virtual void refreshSummaries() = 0;
	virtual bool loading() const = 0;
	/// Meldungen an das Fenster: Statuszeile, Baum geändert, zuschaltbare Funktionen geändert.
	virtual void status(const QString& text) = 0;
	virtual void treeChanged() = 0;
	virtual void featuresChanged() = 0;
};

/// Die Eigenschaften eines Eintrags aus Sicht der Erweiterung.
class RjHostProperties
{
public:
	virtual ~RjHostProperties() = default;
	virtual RjHostBackend* hostBackend() const = 0;
	virtual QWidget* widget() = 0;
	virtual const QJsonObject& data() const = 0;
	virtual bool isDirtyGroup(const QString& group) const = 0;
	virtual void addPluginPage(const QString& key, const char* label, const std::function<QWidget*()>& make) = 0;
	virtual void track(QWidget* w, const QString& group) = 0;
};
