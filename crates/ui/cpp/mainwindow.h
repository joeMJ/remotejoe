#pragma once
// Hauptfenster: Baum links, Tabs rechts (mit Kacheln), Statuszeile.

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

class MainWindow : public QMainWindow
{
	Q_OBJECT
public:
	explicit MainWindow(RjBackend* backend, QWidget* parent = nullptr);
	~MainWindow() override;

	void applyTheme(bool dark);
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
	/// Messwerte der Patchday-Ansicht (längstes Neuzeichnen, Anzahl).
	QString patchdayMeasurement() const;
	QTreeView* tree() const { return m_tree; }
	int treeRowCount() const;
	/// Eigenschaften-Tab öffnen (oder vorhandenen zeigen).
	PropertiesTab* openProperties(const QString& id, const QString& category = QString());
	/// Für Bildschirmfotos: Eigenschaften des ersten Ziels, Hinzufügen-Menü, Entsperren.
	PropertiesTab* openFirstTargetProperties();
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
	QTabWidget* tabs() const { return m_tabs; }

	/// Zeit vom Programmstart bis zum ersten Zeichnen des Fensters.
	static QElapsedTimer startClock;
	qint64 firstPaintMs = -1;

protected:
	void paintEvent(QPaintEvent*) override;
	void closeEvent(QCloseEvent*) override;
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
	void reloadTree();
	void poll();
	void rebuildTabs();
	QWidget* buildTabContent(const QJsonObject& tab);
	void connectSelected(const QString& credential = QString(), bool remember = false);
	void handleConnectResult(const QString& id, const QString& json);
	void showProperties(const QString& id);
	void treeContextMenu(const QPoint& pos);
	void loadGuests(const QModelIndex& index);
	void applyGuests(const QString& target, QJsonObject r);
	void updateJobs();
	void idleSettings();
	void handleEvents(const QString& json);
	void openGuestConsole(const QModelIndex& index);
	void guestAction(const QModelIndex& index, const QString& action, bool confirmed = false);
	/// Tab für Rückfragen beim Verbinden eines Ziels (vorhanden oder neu).
	ConnectPromptTab* promptTab(const QString& target);
	void chooseCredential(ConnectPromptTab* tab, const QJsonArray& choices);
	void addToolTab(ToolTab* t, const QString& title);
	void closeToolTab(ToolTab* t);
	void tabContextMenu(QTabWidget* tabs, const QPoint& pos);
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
	void updateConnInfo();
	void openTerminalSettings();
	void openMacroTab(const QString& id);
	void addMacroShelf();
	void macroContextMenu(const QModelIndex& index, const QPoint& globalPos);
	void deleteMacro(const QString& id, const QString& name);

	RjBackend* m_backend;
	QLineEdit* m_search;
	QTreeView* m_tree;
	QWidget* m_emptyHint = nullptr;
	bool m_macroShelfOpen = false;
	QStandardItemModel* m_model;
	QTabWidget* m_tabs;
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
