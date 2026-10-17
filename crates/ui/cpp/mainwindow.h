#pragma once
// Hauptfenster: Baum links, Tabs rechts (mit Kacheln), Statuszeile.

#include <QtCore/QElapsedTimer>
#include <QtCore/QHash>
#include <QtCore/QJsonObject>
#include <QtCore/QPointer>
#include <QtCore/QTimer>
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
	void detachTabAt(int index);
	void openPatchday(const QStringList& ids);
	void patchdayForFirstTargets(int n);
	QTreeView* tree() const { return m_tree; }
	int treeRowCount() const;
	/// Eigenschaften-Tab öffnen (oder vorhandenen zeigen).
	PropertiesTab* openProperties(const QString& id, const QString& category = QString());
	/// Für Bildschirmfotos: Eigenschaften des ersten Ziels, Hinzufügen-Menü, Entsperren.
	PropertiesTab* openFirstTargetProperties();
	QMenu* popupAddMenu();
	void showUnlock(bool locked);
	QTabWidget* tabs() const { return m_tabs; }

	/// Zeit vom Programmstart bis zum ersten Zeichnen des Fensters.
	static QElapsedTimer startClock;
	qint64 firstPaintMs = -1;

protected:
	void paintEvent(QPaintEvent*) override;
	void closeEvent(QCloseEvent*) override;

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
	void openGuestConsole(const QModelIndex& index);
	void guestAction(const QModelIndex& index, const QString& action);
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

	RjBackend* m_backend;
	QLineEdit* m_search;
	QTreeView* m_tree;
	QStandardItemModel* m_model;
	QTabWidget* m_tabs;
	QTimer m_poll;
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
};
