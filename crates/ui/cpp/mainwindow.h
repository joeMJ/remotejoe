#pragma once
// Hauptfenster: Baum links, Tabs rechts (mit Kacheln), Statuszeile.

#include "rjpluginui.h"

#include <QtCore/QSet>
#include <QtCore/QElapsedTimer>
#include <QtCore/QHash>
#include <QtCore/QJsonObject>
#include <QtCore/QPointer>
#include <QtCore/QTimer>
#include <QtGui/QColor>
#include <QtGui/QStandardItemModel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QMainWindow>
#include <QtWidgets/QTabWidget>
#include <QtWidgets/QTreeView>
#include <cstdint>
#include <functional>
#include <vector>

class QFrame;
class QShortcut;

class RjBackend;
class SessionPane;
class ActionBar;
class PropertiesTab;
class BulkAddTab;
class ToolTab;
class ConnectPromptTab;
class QJsonArray;
class UnlockOverlay;
class QComboBox;
class QMenu;
class QAction;
class QEvent;
class QMouseEvent;
class QEnterEvent;
class QMenuBar;
class TitleBar;

/// Senkrechte Registerkarte „Ziele“ am linken Rand (gelöste Stecknadel): Überfahren klappt den Zielbereich auf.
class RailTab : public QWidget
{
public:
	RailTab(const QString& text, QWidget* parent);
	QSize sizeHint() const override { return QSize(28, 120); }
	std::function<void()> onEnter, onLeave, onClick;

protected:
	void paintEvent(QPaintEvent*) override;
	void enterEvent(QEnterEvent*) override;
	void leaveEvent(QEvent*) override;
	void mousePressEvent(QMouseEvent*) override;

private:
	QString m_text;
};

/// Runde Fensterecken unter Linux: setzt an den Fensterteilen, die eine Ecke berühren, abgerundete Masken und
/// passt sie bei Größen- und Zustandswechseln an. Das Fenster selbst zeichnet Rand, Fläche und Schatten.
class RoundCorners : public QObject
{
	Q_OBJECT
public:
	static constexpr int kRadius = 12;
	explicit RoundCorners(QMainWindow* window);
	bool eventFilter(QObject* o, QEvent* e) override;

private:
	void update();
	QMainWindow* m_window;
	QTimer m_timer;
	QSet<QWidget*> m_watched;
	QList<QWidget*> m_grips;
};

class MainWindow : public QMainWindow, public RjHostWindow
{
	Q_OBJECT
public:
	explicit MainWindow(RjBackend* backend, QWidget* parent = nullptr);
	~MainWindow() override;
	TitleBar* titleBar() const { return m_titleBar; }

	void applyTheme(bool dark);
	void copyCredential(const QString& id, const QString& kind);
	/// Fensterlage aus der Konfiguration herstellen (vor dem Zeigen) bzw. beim Beenden merken.
	void restoreWindowState();
	void tryKeyringUnlock();
	void forgetKeyring();
	void saveWindowState();
	/// Platzsparmodus: Menü im Hamburger-Knopf, Tabs in der Titelzeile.
	void setCompact(bool on);
	bool compact() const { return m_compact; }
	/// Baum links ein- oder ausklappen.
	void setTreeVisible(bool on);
	/// Favoriten, Stecknadel und Verzögerungen aus der Darstellung übernehmen (Start und nach Änderungen).
	void applyTreePrefs();
	/// Fenster auf den sichtbaren Bildschirmbereich begrenzen und zurück ins Bild holen.
	void fitToScreen();
	/// Größe, Lage und Baumbreite auf die Voreinstellung setzen (Ansicht → Fenster zurücksetzen).
	void resetWindow();
	void setFilter(const QString& text);
	/// Ersten Hypervisor im Baum aufklappen (lädt die Gäste).
	void expandFirstHypervisor();
	QStringList expandedIdsForCheck() const; // Prüfbild: aufgeklappte Ordner (Kennungen, sortiert)
	QWidget* detachedWindow() const;
	void showCredentialChoiceForScreenshot();
	/// Für Bildschirmfotos: Host-Schlüssel-Rückfrage in der ersten Sitzung.
	QWidget* showHostKeyQuestionForScreenshot(bool changed);
	void openImport(const QString& path, const QJsonObject& report, const QString& summary);
	void detachTabAt(int index);
	void openPatchday(const QStringList& ids);
	void patchdayForFirstTargets(int n);
	/// Vorschau „Gleiche Adressen zusammenfassen“ für den ersten Ordner (Bildschirmfotos).
	void mergeForFirstFolder();
	/// Extras → „Geteilte Zugangsdaten prüfen“.
	void openSharedCheck();
	void openMerge(const QStringList& ids, const QString& title, bool manual = false);
	/// Messwerte der Patchday-Ansicht (längstes Neuzeichnen, Anzahl).
	QString patchdayMeasurement() const;
	QTreeView* tree() const override { return m_tree; }
	int treeRowCount() const;
	/// Eigenschaften-Tab öffnen (oder vorhandenen zeigen).
	PropertiesTab* openProperties(const QString& id, const QString& category = QString());
	/// Anmeldeobjekt öffnen (`id` leer: neu im Ordner `folder` bzw. im Ordner des Ziels `folder`).
	void openLogin(const QString& id, const QString& folder);
	/// Gleiche Zugangsdaten zusammenführen (leer: ganzer Baum).
	void openCredMerge(const QStringList& scope);
	/// Übersicht eines Ordners (ein Tab, beim nächsten Ordner neu befüllt).
	void openFolderOverview(const QModelIndex& folder, bool activate = true);
	/// Kontextmenü des Baums: Aufklappzustand als Start speichern / zurücksetzen.
	void addTreeStartActions(class QMenu* menu);
	/// Über einen Zugangsweg verbinden (`way`: „main“, Weg-ID oder `ssh:<IP>`).
	void connectWay(const QJsonObject& subject, const QString& way) override;
	/// Objektübersicht des Ziels (ein Tab für alle, Einfachklick im Baum).
	void openOverview(const QJsonObject& subject, const QIcon& icon = QIcon()) override;
	/// Ist `id` ein Anmeldeobjekt im Baum?
	bool isLogin(const QString& id) const;
	/// Für Bildschirmfotos: Eigenschaften des ersten Ziels, Hinzufügen-Menü, Entsperren.
	PropertiesTab* openFirstTargetProperties(const QString& protocol = QStringLiteral("rdp"));
	QMenu* popupAddMenu();
	BulkAddTab* openBulkAdd(const QString& parentId);
	void showUnlock(bool locked);
	void showNewVault();
	/// Nur für Bildschirmfotos: Beispielmakro anlegen, Fach öffnen, Editor zeigen.
	void macrosForScreenshot();
	/// Nur für Bildschirmfotos: Terminal-Einstellungen mit gewähltem Schema öffnen.
	void terminalSettingsForScreenshot(const QString& scheme);
	/// Nur für Bildschirmfotos: leerer Baum mit Hinweis.
	void showEmptyTreeForScreenshot();
	/// Wählt den ersten Tab mit einer Web-Sitzung; `false`, wenn es keinen gibt.
	bool showWebTabForScreenshot();
	/// Nur für Bildschirmfotos: Darstellungs-Tab öffnen; Über-Dialog erzeugen (nicht modal gezeigt).
	void appearanceForScreenshot() { openAppearance(); }
	/// Extras → Einstellungen beim Bereich `section` (lock, keys, terminal, appearance, snmp).
	void settingsForScreenshot(const QString& section) { openSettings(section); }
	/// Bildschirmfoto: Übersicht des ersten Unterordners (→ erster Ordner).
	void folderForScreenshot() { openFolderOverview(m_model->index(0, 0, m_model->index(0, 0))); }
	QWidget* aboutForScreenshot();
	/// Nur für Bildschirmfotos: Stecknadel lösen bzw. anheften, Zielbereich aufklappen.
	void flyoutForScreenshot(bool unpinned, bool open);
	RjBackend* backendForScreenshot() const { return m_backend; }

	/// Beim ersten Start einer neuen Version die Neuigkeiten zeigen (nicht bei einer neuen Installation).
	void showNewsIfNew();
	/// Releasenotes der Erweiterungen zeigen, wenn ein neuer Eintrag da ist (einmal je Stand).
	void showExtNewsIfNew();
	/// Hinweise beim allerersten Start (Pfeile auf Menü und „Hinzufügen“), solange es kein Ziel gibt und sie nicht
	/// abgeschaltet sind; `force` zeigt sie immer (Bildschirmfotos). Liefert die Hinweisfläche.
	QWidget* showFirstHints(bool force = false);
	/// Update-Tab mit einer Prüfung gegen `base` (lokale Attrappe).
	void updateForScreenshot(const QString& base);
	QTabWidget* tabs() const override { return m_tabs; }

	// Host-Schnittstelle für Erweiterungen (plugin/rjpluginui.h): Baum, Tabs, Leiste, Bearbeiten über den Kern.
	RjBackend* backend() const { return m_backend; }
	QStandardItemModel* model() const override { return m_model; }
	ActionBar* bar() const { return m_bar; }
	void reloadTree() override;
	void addToolTab(ToolTab* t, const QString& title);
	void closeToolTab(ToolTab* t);
	QString edit(const QJsonObject& cmd) override;
	QString currentId() const override;
	QStringList selectedIds() const override;
	void selectId(const QString& id) override;
	void renameSelected() override;
	void deleteSelected() override;
	void showEditResult(const QJsonObject& r, const QString& okMessage = QString()) override;

	// RjHostWindow (Oberfläche einer geladenen Erweiterung).
	RjHostBackend* hostBackend() const override;
	RjPluginUi* pluginUi(const QString& id) override;
	QWidget* widget() override { return this; }
	QStatusBar* statusBar() const override { return QMainWindow::statusBar(); }
	void addToolTab(QWidget* tab, const QString& title) override;
	void closeToolTab(QWidget* tab) override;
	void showProperties(const QString& id, const QString& category = QString()) override { openProperties(id, category); }
	void ask(const QString& text, const QStringList& buttons, const std::function<void(int)>& done) override;
	void prompt(const QString& text, const QString& preset, const std::function<void(const QString&)>& done) override;
	RjHostOverview* findOverview() override;

	/// Zeit vom Programmstart bis zum ersten Zeichnen des Fensters.
	static QElapsedTimer startClock;
	qint64 firstPaintMs = -1;

public slots:
	/// Tab „Neuigkeiten“ (Releasenotes, neueste oben).
	void openNews(bool ext = false);

protected:
	void paintEvent(QPaintEvent*) override;
	void closeEvent(QCloseEvent*) override;
	void mouseMoveEvent(QMouseEvent*) override;
	void updateEdgeCursor(const QPoint& global);
	void mousePressEvent(QMouseEvent*) override;
	void changeEvent(QEvent*) override;
	bool eventFilter(QObject*, QEvent*) override;

private slots:
	/// Meldung von logind (D-Bus): `true` vor dem Standby, `false` nach dem Aufwachen.
	void onPrepareForSleep(bool sleeping);
	/// Signale der Tabs von Erweiterungen (nach Namen verbunden).
	void pluginTabIndexChanged();
	void pluginTabStatus(const QString& m);
	void pluginTabClose();

public:
	/// Aufwachen aus dem Standby verarbeiten (Meldung des Systems).
	void systemResumed();
	/// Sperren (von Hand oder nach Leerlauf): Abdeckung zeigen, Fenster verbergen.
	void lockNow(bool alreadyLocked);

	// Beiträge der Module (nur mit dem jeweiligen Modul gebaut).
#ifndef Q_MOC_RUN
	/// Lizenz aus der Schlüsseldatenbank übernommen: Hinweis mit Neustart (licensepage.cpp).
	void licenseNoticeAfterUnlock();
	QFrame* m_licenseNotice = nullptr;
public:
	/// Absturzwächter der Erweiterungen (nach dem Zeigen des Hauptfensters): läuft dieser Start ohne Erweiterungen,
	/// eine Leiste mit Grund und „Mit Erweiterungen neu starten“ / „Erweiterungen aktualisieren“; sonst nach 30 s
	/// stabilem Lauf die Startmarke entfernen.
	void pluginStartGuard();
private:
	QFrame* m_safeModeNotice = nullptr;
	/// remotejoe beenden und neu starten (wie nach einem Update); false, wenn das Fenster nicht schließt oder der
	/// Start scheitert.
	bool restartProgram();
#endif

public:
	/// Beiträge der Erweiterungen zu Menüs und Klicks (rjhost.h); angemeldet in RjPluginUi::attach().
	using ModuleHooks = RjModuleHooks;
	ModuleHooks m_modules;
	RjModuleHooks& hooks() override { return m_modules; }

public:
	/// Bildschirmfotos der Module (Vorführung).
	/// Einstellungen öffnen (Bereich `section`, z. B. „keys“).
	void openSettings(const QString& section);
	void moduleScreenshots(const std::function<void(int)>& pump, const std::function<void(const QString&)>& shot)
	{
		for (const auto& h : m_modules.screenshots)
			h(pump, shot);
	}

private:

	void buildMenus();
	/// Font-Awesome-Symbole an die Menüeinträge setzen (auch im Hamburger-Menü).
	void applyMenuIcons();
	bool m_round = false;
	RoundCorners* m_roundCorners = nullptr;
	/// Ordner „Favoriten“ oben in den Baum setzen (Einstellung „Favoriten anzeigen“).
	void addFavoritesFolder();
	/// Stecknadel: angeheftet (Baum im Teiler) oder gelöst (Registerkarte „Ziele“, Aufklappen über den Tabs).
	void setPinned(bool on, bool save);
	void openFlyout();
	void closeFlyout();
	void placeFlyout();
	void updatePinButton();
	bool m_pinned = true;
	bool m_flyOpen = false;
	bool m_showFavorites = true;
	bool m_favOpen = true;
	QColor m_starColor = QColor("#d4a017");
	int m_flyOpenMs = 300;
	int m_flyCloseMs = 500;
	QTimer m_flyOpenTimer;
	QTimer m_flyCloseTimer;
	QWidget* m_center = nullptr;
	RailTab* m_rail = nullptr;
	class QToolButton* m_pinButton = nullptr;
	void placeBrand();
	void syncStripHeight();
	void updateNotice();
	void screenLayoutChanged();
	/// Suchtext anwenden, ohne den Baum neu aufzubauen: Nicht passende Zeilen werden ausgeblendet.
	void applySearch();
	/// Punkt am Symbol der Ziele mit offenem Tab (gefüllt: verbunden, Ring: nicht verbunden).
	void updateSessionMarks();
	void poll();
	void rebuildTabs();
	QWidget* buildTabContent(const QJsonObject& tab);
	/// Alle markierten Ziele verbinden (bei nur einem: das aktuelle).
	void connectSelected(const QString& credential = QString(), bool remember = false);
	/// Nur das aktuelle Ziel verbinden.
	void connectCurrent(const QString& credential = QString(), bool remember = false);
	/// Gestaffeltes Verbinden mehrerer Ziele (`ids` oder Ordner `id`), ab `kConnectWithoutAsking` mit Rückfrage.
	void connectMany(const QJsonObject& which);
	bool confirmManyTargets(int n);
	static constexpr int kConnectWithoutAsking = 5;
	void handleConnectResult(const QString& id, const QString& json);
	void showProperties(const QString& id);
	void treeContextMenu(const QPoint& pos);
	void loadGuests(const QModelIndex& index);
	void applyGuests(const QString& target, QJsonObject r);
	void updateJobs();
	void handleEvents(const QString& json);
	void applyPageTitle(SessionPane* p);
	void openUpdateTab();
	void openGuestConsole(const QModelIndex& index);
	void guestAction(const QModelIndex& index, const QString& action, bool confirmed = false);
	/// Tab für Rückfragen beim Verbinden eines Ziels (vorhanden oder neu).
	ConnectPromptTab* promptTab(const QString& target);
	void chooseCredential(ConnectPromptTab* tab, const QJsonArray& choices);
	void tabContextMenu(QTabWidget* tabs, const QPoint& pos);
	/// Administrativer Tastaturmodus einer RDP-Sitzung ein/aus (`reason`: Hinweis beim automatischen Abschalten).
	void setAdminKeys(SessionPane* pane, bool on, const QString& reason = QString());
	/// Tastatursymbol neben dem Tab-Symbol aller Tabs mit eingeschaltetem Modus.
	void updateAdminIndicators();
	void configureTabBar(QTabWidget* tabs);
	void buildAddMenu(QMenu* menu, const QString& parentId);
	QString currentParentId() const;
	void moveSelected();
	void duplicateSelected();
	void toggleFavorite();
	void reorderSelected(int delta);
	void undo();
	void connectAll(const QString& mode);
	void reloadViews();
	void viewCommand(const QString& what);
	void saveAll(const QString& choice = QString());
	SessionPane* pane(std::uint64_t session);
	std::uint64_t currentTab() const;
	std::uint64_t currentSession() const;
	QSize contentSize(bool terminal) const;
	void updateStatus();
	void updateEmptyHint();
	/// Logo unten rechts im Tab-Bereich, solange kein Tab offen ist.
	void updateLogo();
	/// Liegt der Tastaturfokus in einer Sitzung (RDP, VNC, Terminal, Web)?
	bool focusInSession() const;
	/// Darf Strg+Umschalt+W gerade den Tab schließen (Einstellung „close_tab_key“)?
	bool closeTabKeyActive() const;
	/// Strg+F geht an die Suche über dem Baum (sonst an die Sitzung); eine Web-Sitzung sucht immer in der Seite.
	bool findKeyActive() const;
	/// Tab-Wechsel per Tastenfolge erlaubt (Modus wie bei Strg+Umschalt+W)?
	bool tabSwitchKeyActive() const;
	/// Tastenfolgen für den Tab-Wechsel setzen (leer = keine).
	void applyTabSwitchKeys(const QString& mode, const QString& prev, const QString& next);
	/// Aktiven Tab um `step` verschieben (mit Umlauf).
	void stepTab(int step);
	/// Fokus in einer Web-Sitzung: deren Bereich, sonst nullptr.
	SessionPane* focusedWebPane() const;
	void updateConnInfo();
	void openTerminalSettings();
	void openAppearance();
	void openExport();
	/// Schema und Fokusrahmen aus den Einstellungen anwenden.
	void applyAppearance();
	void openMacroTab(const QString& id);
	void addMacroShelf();
	void macroContextMenu(const QModelIndex& index, const QPoint& globalPos);
	void deleteMacro(const QString& id, const QString& name);

	RjBackend* m_backend;
	QLineEdit* m_search;
	bool m_edgeOverride = false;
	bool m_treeLoaded = false;
	/// Noch nie entsperrt: beim ersten Entsperren gilt der Startzustand des Baums.
	bool m_firstUnlock = true;
	/// Aufklappen durch das Programm (Wiederherstellen, Suche): dabei keine Gäste laden.
	bool m_autoExpand = false;
	/// Hypervisor-Knoten, deren Gäste noch nicht geladen sind, wieder zuklappen.
	void collapseUnloadedHypervisors();
	/// Suche aktiv (Zeilen ausgeblendet); beim Verlassen wird der Baum einmal neu aufgebaut.
	bool m_searchApplied = false;
	/// Suche: mit Enter ausgelöst (gilt auch unter der Mindestzahl Zeichen), Schwellen aus den Einstellungen.
	bool m_searchForced = false;
	bool m_searchOnEnter = false;
	int m_searchMinChars = 4;
	class QLabel* m_searchHint = nullptr;
	/// Wirksamer Suchtext (leer = keine Suche: Feld leer oder unter der Schwelle ohne Enter).
	QString searchText() const;
	void updateSearchHint();
	/// Enter im Suchfeld: sofort suchen (auch unter der Schwelle), dann in den Baum.
	void searchEnter();
	bool m_keepExpanded = false; // reloadTree während einer Suche: gemerkten Aufklappzustand behalten
	QSet<QString> m_expandedIds;
	QTimer* m_clipTimer = nullptr;
	QByteArray m_clipHash;
	QTreeView* m_tree = nullptr;
	class QSplitter* m_split = nullptr;
	class QLabel* m_emptyLabel = nullptr;
	class QPushButton* m_emptyButton = nullptr;
	TitleBar* m_titleBar = nullptr;
	QMenuBar* m_menuBar = nullptr;
	// Platzsparmodus, Baum und Fußzeile.
	bool m_compact = false;
	bool m_treeHidden = false;
	int m_treeWidth = 280;
	QWidget* m_leftPane = nullptr;
	QWidget* m_leftInner = nullptr;
	QWidget* m_brandSlot = nullptr;
	class QVBoxLayout* m_brandSlotLay = nullptr;
	QMenu* m_burgerMenu = nullptr;
	QAction* m_compactAction = nullptr;
	class QAbstractButton* m_compactSwitch = nullptr;
	class QToolButton* m_treeButton = nullptr;
	class QLabel* m_noticeLabel = nullptr;
	/// Zuletzt übergangener Schlüsselfehler einer Hintergrundabfrage (Fußzeile, rot, eine Minute).
	QString m_keyWarning;
	QTimer m_fitTimer;
	QWidget* m_emptyHint = nullptr;
	class QLabel* m_logo = nullptr;
	/// Strg+Umschalt+W: „outside“, „always“ oder „never“.
	QString m_closeTabKey = QStringLiteral("outside");
	QString m_findKey = QStringLiteral("outside");
	QString m_tabSwitchKey = QStringLiteral("outside");
	QShortcut* m_tabPrevShortcut = nullptr;
	QShortcut* m_tabNextShortcut = nullptr;
	bool m_macroShelfOpen = false;
	QStandardItemModel* m_model = nullptr;
	QTabWidget* m_tabs = nullptr;
	QTimer m_poll;
	// Fußzeile: Verbindungstyp, Ziel, Beginn und Benutzer der vorderen Sitzung.
	QWidget* m_connInfo = nullptr;
	class QLabel* m_ciProtoIcon = nullptr;
	class QLabel* m_ciProto = nullptr;
	class QLabel* m_ciTargetIcon = nullptr;
	class QLabel* m_ciTarget = nullptr;
	class QLabel* m_ciSinceIcon = nullptr;
	class QLabel* m_ciSince = nullptr;
	class QLabel* m_ciUserIcon = nullptr;
	class QLabel* m_ciUser = nullptr;
	QTimer m_ciTimer;
	QColor m_ciColor;
	QTimer m_filterTimer;
	QHash<std::uint64_t, SessionPane*> m_panes;
	QHash<std::uint64_t, QPointer<QWidget>> m_windows;
	QHash<std::uint64_t, QTabWidget*> m_windowTabs;
	QJsonObject m_workspace;
	QSet<std::uint64_t> m_dirtyTerminals;
	ActionBar* m_bar = nullptr;
	QComboBox* m_viewBox = nullptr;
	UnlockOverlay* m_overlay = nullptr;
	QAction* m_undoAction = nullptr;
	QWidget* m_jobsBox = nullptr;
	class QLabel* m_jobsLabel = nullptr;
	QHash<qulonglong, QString> m_jobTargets;
	qint64 m_lastActivity = 0;
	QElapsedTimer m_activityClock;
};
