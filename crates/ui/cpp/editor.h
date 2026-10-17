#pragma once
// Bearbeiten im Hauptfenster: Leiste für Rückfragen, Eigenschaften-Tab und
// Abdeckung zum Entsperren. Keine eigenen Fenster der obersten Ebene.

#include <QtCore/QJsonArray>
#include <QtCore/QJsonObject>
#include <QtCore/QSet>
#include <QtWidgets/QFrame>
#include <QtWidgets/QWidget>
#include <functional>

class RjBackend;
class QButtonGroup;
class QCheckBox;
class QComboBox;
class QLabel;
class QLineEdit;
class QListWidget;
class QPlainTextEdit;
class QPushButton;
class QStackedWidget;
class QTableWidget;
class QTreeWidget;

/// Text aus der Ressourcendatei.
QString rjText(RjBackend* b, const char* key);

/// Leiste innerhalb des Fensters für Bestätigungen und kurze Eingaben.
class ActionBar : public QFrame
{
	Q_OBJECT
public:
	explicit ActionBar(RjBackend* backend, QWidget* parent = nullptr);
	/// Frage mit eigenen Schaltflächen; Rückruf mit deren Index, -1 bei Abbruch.
	void ask(const QString& text, const QStringList& buttons, std::function<void(int)> done);
	/// Eingabezeile mit Vorbelegung.
	void prompt(const QString& text, const QString& value, std::function<void(const QString&)> done);
	/// Auswahl aus einer Liste (Anzeige, Wert).
	void choose(const QString& text, const QList<QPair<QString, QString>>& items, std::function<void(const QString&)> done,
	            const QString& checkText = QString(), std::function<void(const QString&, bool)> doneCheck = nullptr);
	void dismiss();
	QLineEdit* line() const { return m_line; }
	QComboBox* combo() const { return m_combo; }

protected:
	void keyPressEvent(QKeyEvent*) override;

private:
	void reset(const QString& text);
	RjBackend* m_backend;
	QLabel* m_label;
	QLineEdit* m_line;
	QComboBox* m_combo;
	QCheckBox* m_check;
	QList<QPushButton*> m_buttons;
	QPushButton* m_cancel;
	std::function<void(int)> m_done;
};

/// Eigenschaften eines Ordners oder Ziels als Tab.
class PropertiesTab : public QWidget
{
	Q_OBJECT
public:
	PropertiesTab(RjBackend* backend, const QString& id, QWidget* parent = nullptr);
	QString nodeId() const { return m_id; }
	QString title() const;
	bool isDirty() const { return !m_dirty.isEmpty(); }
	/// Schließen; bei ungespeicherten Änderungen wird in der Leiste gefragt.
	void requestClose(std::function<void()> close);
	void selectCategory(const QString& key);
	void reload();
	ActionBar* bar() const { return m_bar; }
	/// Beim Sperren: eingegebene Kennwörter verwerfen.
	void wipeSecrets();

signals:
	/// Der Index hat sich geändert (Baum neu laden).
	void indexChanged();
	void titleChanged();
	void status(const QString& text);

private:
	void build();
	QWidget* pageGeneral();
	QWidget* pageConnection();
	QWidget* pageCredentials();
	QWidget* pageTunnel();
	QWidget* pageTasks();
	QWidget* pageClipboard();
	QWidget* pageOrganisation();
	QWidget* pageParent();
	void addPage(const QString& key, const char* label, QWidget* page);
	void track(QWidget* w, const QString& group);
	void markDirty(const QString& group);
	void clearDirty();
	void updateCredentialMode();
	void showErrors(const QJsonObject& errors);
	QJsonObject collect() const;
	void save();
	void handleSaveResult(const QJsonObject& r);
	void applyToChildren(const QString& kind, const QString& label);
	void saveTemplate();
	QString edit(const QJsonObject& cmd);

	RjBackend* m_backend;
	QString m_id;
	QJsonObject m_data;
	QSet<QString> m_dirty;
	bool m_loading = false;
	QTreeWidget* m_cats = nullptr;
	QStackedWidget* m_pages = nullptr;
	ActionBar* m_bar = nullptr;
	QLabel* m_dirtyLabel = nullptr;
	QLabel* m_status = nullptr;
	QHash<QString, QLabel*> m_errors;
	// Allgemein
	QLineEdit* m_name = nullptr;
	QLabel* m_nameWarn = nullptr;
	QLineEdit* m_desc = nullptr;
	QCheckBox* m_fav = nullptr;
	// Verbindung
	QLineEdit* m_host = nullptr;
	QLabel* m_hostWarn = nullptr;
	QLineEdit* m_port = nullptr;
	QHash<QString, QWidget*> m_opts;
	QComboBox* m_psrp = nullptr;
	QComboBox* m_psrpAuth = nullptr;
	QLineEdit* m_psrpPort = nullptr;
	QCheckBox* m_psrpVerify = nullptr;
	QComboBox* m_snmp = nullptr;
	// Zugangsdaten
	QButtonGroup* m_credMode = nullptr;
	QLineEdit* m_user = nullptr;
	QLineEdit* m_password = nullptr;
	QLabel* m_pwWarn = nullptr;
	QComboBox* m_entry = nullptr;
	QComboBox* m_named = nullptr;
	QLabel* m_credEffective = nullptr;
	QWidget* m_directBox = nullptr;
	QWidget* m_entryBox = nullptr;
	QWidget* m_namedBox = nullptr;
	// Tunnel
	QComboBox* m_tunnel = nullptr;
	// Aufgaben und Makros
	QListWidget* m_taskList = nullptr;
	QLineEdit* m_taskName = nullptr;
	QLineEdit* m_taskKind = nullptr;
	QPlainTextEdit* m_taskBody = nullptr;
	// Zwischenablage
	QComboBox* m_clip = nullptr;
	// Organisation
	QPlainTextEdit* m_notes = nullptr;
	QTableWidget* m_fields = nullptr;
	// Übergeordneter Ordner
	QComboBox* m_parent = nullptr;
};

/// SNMP-Geräte verwalten (Tab): Community bzw. v3-Benutzer mit getrennten
/// Kennwörtern für Authentisierung und Verschlüsselung.
class SnmpTab : public QWidget
{
	Q_OBJECT
public:
	explicit SnmpTab(RjBackend* backend, QWidget* parent = nullptr);
	void reload();
	void wipeSecrets();

signals:
	void status(const QString& text);

private:
	void showDevice(int row);
	void save();
	QString edit(const QJsonObject& cmd);
	RjBackend* m_backend;
	QJsonArray m_devices;
	QListWidget* m_list;
	QLineEdit* m_name;
	QLineEdit* m_host;
	QLineEdit* m_port;
	QComboBox* m_version;
	QLineEdit* m_community;
	QLineEdit* m_user;
	QComboBox* m_auth;
	QLineEdit* m_authPw;
	QComboBox* m_priv;
	QLineEdit* m_privPw;
	QLabel* m_message;
	QString m_current;
};

/// Abdeckung des Hauptfensters zum Entsperren der KDBX.
class UnlockOverlay : public QWidget
{
	Q_OBJECT
public:
	explicit UnlockOverlay(RjBackend* backend, QWidget* parent);
	/// `locked`: Sperre nach Leerlauf oder von Hand – kein Abbrechen möglich.
	void open(const QString& path, bool locked);
	void setBusy(bool busy, const QString& message = QString());

signals:
	void unlockRequested(const QString& path, const QString& password, const QString& keyFile);

protected:
	void paintEvent(QPaintEvent*) override;
	bool eventFilter(QObject*, QEvent*) override;

private:
	void submit();
	RjBackend* m_backend;
	QLineEdit* m_path;
	QLineEdit* m_password;
	QLineEdit* m_keyFile;
	QPushButton* m_ok;
	QPushButton* m_cancel;
	QLabel* m_message;
};
