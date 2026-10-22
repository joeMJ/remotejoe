#pragma once
// Hauptfenster: Baum links, Tabs rechts (mit Kacheln), Statuszeile.

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
class QMenuBar;
class TitleBar;

class MainWindow : public QMainWindow
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
	/// Fenster auf den sichtbaren Bildschirmbereich begrenzen und zurück ins Bild holen.
	void fitToScreen();
	/// Größe, Lage und Baumbreite auf die Voreinstellung setzen (Ansicht → Fenster zurücksetzen).
	void resetWindow();
	void setFilter(const QString& text);
	/// Ersten Hypervisor im Baum aufklappen (lädt die Gäste).
	void expandFirstHypervisor();
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
	QTreeView* tree() const { return m_tree; }
	int treeRowCount() const;
	/// Eigenschaften-Tab öffnen (oder vorhandenen zeigen).
	PropertiesTab* openProperties(const QString& id, const QString& category = QString());
	/// Anmeldeobjekt öffnen (`id` leer: neu im Ordner `folder` bzw. im Ordner des Ziels `folder`).
	void openLogin(const QString& id, const QString& folder);
	/// Aruba-Werkzeuge (ap_check) für ein Ziel oder einen Ordner.
	void openApTools(const QString& id, const QString& name, const QString& tool);
	/// Aruba-Log (system/security, wahlweise gefiltert) am VC oder an einem AP anzeigen.
	void openArubaLog(const QJsonObject& subject, const QString& ip, const QString& where, const QString& kind, bool filtered);
	/// Über einen Zugangsweg verbinden (`way`: „main“, Weg-ID oder `ssh:<IP>`).
	void connectWay(const QJsonObject& subject, const QString& way);
	/// Objektübersicht des Ziels (ein Tab für alle, Einfachklick im Baum).
	void openOverview(const QJsonObject& subject, const QIcon& icon = QIcon());
	/// Ist `id` ein Anmeldeobjekt im Baum?
	bool isLogin(const QString& id) const;
	/// Für Bildschirmfotos: Eigenschaften des ersten Ziels, Hinzufügen-Menü, Entsperren.
	/// Bildschirmfotos: Aruba-Werkzeuge mit erfundenem Cluster am ersten SSH-Ziel; Übersicht oder AP-Seite.
	QWidget* arubaForScreenshot(bool apPage);
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
	QWidget* aboutForScreenshot();
	/// Update-Tab mit einer Prüfung gegen `base` (lokale Attrappe).
	void updateForScreenshot(const QString& base);
	QTabWidget* tabs() const { return m_tabs; }

	/// Zeit vom Programmstart bis zum ersten Zeichnen des Fensters.
	static QElapsedTimer startClock;
	qint64 firstPaintMs = -1;

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

public:
	/// Aufwachen aus dem Standby verarbeiten (Meldung des Systems).
	void systemResumed();
	/// Sperren (von Hand oder nach Leerlauf): Abdeckung zeigen, Fenster verbergen.
	void lockNow(bool alreadyLocked);

private:
	void buildMenus();
	void placeBrand();
	void syncStripHeight();
	void updateNotice();
	void screenLayoutChanged();
	void reloadTree();
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
	void addToolTab(ToolTab* t, const QString& title);
	void closeToolTab(ToolTab* t);
	void tabContextMenu(QTabWidget* tabs, const QPoint& pos);
	void configureTabBar(QTabWidget* tabs);
	void buildAddMenu(QMenu* menu, const QString& parentId);
	QString edit(const QJsonObject& cmd);
	QString currentId() const;
	QString currentParentId() const;
	QStringList selectedIds() const;
	void selectId(const QString& id);
	void renameSelected();
	void moveSelected();
	void deleteSelected();
	void duplicateSelected();
	void toggleFavorite();
	void reorderSelected(int delta);
	void undo();
	void connectAll(const QString& mode);
	void reloadViews();
	void viewCommand(const QString& what);
	void saveAll(const QString& choice = QString());
	void showEditResult(const QJsonObject& r, const QString& okMessage = QString());
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
	void updateConnInfo();
	void openTerminalSettings();
	void openAppearance();
	void openSettings(const QString& section);
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
	/// Aufklappen durch das Programm (Wiederherstellen, Suche): dabei keine Gäste laden.
	bool m_autoExpand = false;
	/// Hypervisor-Knoten, deren Gäste noch nicht geladen sind, wieder zuklappen.
	void collapseUnloadedHypervisors();
	/// Suche aktiv (Zeilen ausgeblendet); beim Verlassen wird der Baum einmal neu aufgebaut.
	bool m_searchApplied = false;
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
	QTimer m_fitTimer;
	QWidget* m_emptyHint = nullptr;
	class QLabel* m_logo = nullptr;
	/// Strg+Umschalt+W: „outside“, „always“ oder „never“.
	QString m_closeTabKey = QStringLiteral("outside");
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
