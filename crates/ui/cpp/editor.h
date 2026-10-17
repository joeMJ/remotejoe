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
class QFormLayout;
class QFontComboBox;
class QSpinBox;
class QLabel;
class QLineEdit;
class QListWidget;
class QPlainTextEdit;
class QPushButton;
class QStackedWidget;
class QTableWidget;
class QTreeWidget;
class QTimer;

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
	/// Hinweis oder Fehlermeldung mit „Schließen“.
	void inform(const QString& text);
	void dismiss();
	QLabel* label() const { return m_label; }
	QLineEdit* line() const { return m_line; }
	QComboBox* combo() const { return m_combo; }

signals:
	/// Mit „Abbrechen“ oder Escape geschlossen.
	void cancelled();

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

/// Leiste für Sicherheitsrückfragen (Host-Schlüssel, Zertifikate): voller
/// Fingerabdruck und Schlüsseltyp, Standard ist Ablehnen, bei geändertem
/// Schlüssel Warnung mit altem und neuem Fingerabdruck. Annehmen nur durch
/// ausdrückliche Handlung; ohne Antwort wird nach der Frist abgelehnt.
class SecurityBar : public QFrame
{
	Q_OBJECT
public:
	explicit SecurityBar(RjBackend* backend, QWidget* parent = nullptr);
	/// `q` ist die Kennung der Frage; `e` enthält text, detail, info
	/// (kind, host, port, key_type, fingerprint, previous) und timeout_secs.
	void showQuestion(int q, const QJsonObject& e, std::function<void(bool)> done);
	/// Die Frage wurde anderweitig erledigt (Frist, Sitzung beendet).
	void closeQuestion(int q, const QString& reason);
	/// Alle offenen Fragen ablehnen (z. B. beim Schließen des Tabs).
	void rejectAll();
	bool isAsking() const { return m_current >= 0; }

signals:
	/// Frage ohne Antwort beendet (`timeout`, `closed`, `ended`).
	void closedWithoutAnswer(const QString& reason);

protected:
	void keyPressEvent(QKeyEvent*) override;

private:
	struct Pending
	{
		int q;
		QJsonObject e;
		std::function<void(bool)> done;
	};
	void next();
	void finish(bool accept);
	void tick();
	RjBackend* m_backend;
	QLabel* m_title;
	QLabel* m_details;
	QLabel* m_countdown;
	QCheckBox* m_checked;
	QPushButton* m_reject;
	QPushButton* m_accept;
	QTimer* m_timer;
	int m_current = -1;
	int m_left = 0;
	std::function<void(bool)> m_done;
	QList<Pending> m_queue;
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
	/// Änderungen speichern (Strg+S, wenn der Tab vorn liegt); der Tab bleibt offen.
	void saveNow() { save(); }
	/// Schaltfläche „Speichern“: speichern und bei Erfolg den Tab schließen.
	void saveAndClose();

signals:
	/// Der Index hat sich geändert (Baum neu laden).
	void indexChanged();
	/// Gespeichert über die Schaltfläche „Speichern“: der Tab kann geschlossen werden.
	void saveClosed();
	void titleChanged();
	void status(const QString& text);

private:
	bool m_closeAfter = false;
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

/// Tab für eine Aufgabe im Hauptfenster (Mehrere Ziele, Import-Vorschau,
/// Patchday). Schließen verwirft eingegebene Geheimnisse.
class ToolTab : public QWidget
{
	Q_OBJECT
public:
	using QWidget::QWidget;
	virtual void wipeSecrets() {}

signals:
	/// Der Index hat sich geändert (Baum neu laden).
	void indexChanged();
	void status(const QString& text);
	/// Der Tab ist erledigt und kann geschlossen werden.
	void closeRequested();
};

/// „Nach Updates suchen“: Quelle einstellen, prüfen, Änderungstext,
/// herunterladen und installieren, Neustart anbieten. Kein Dialogfenster.
class UpdateTab : public ToolTab
{
	Q_OBJECT
public:
	explicit UpdateTab(RjBackend* backend, QWidget* parent = nullptr);
	/// Ergebnis einer Hintergrundaufgabe (`update_check`, `update_install`).
	void handleResult(const QString& kind, const QJsonObject& r);
	/// Zustand neu laden (z. B. nach dem Entsperren).
	void refresh();
	void check();

private:
	void saveSettings();
	void install(bool confirm);
	void setBusy(bool busy, const QString& text = QString());
	RjBackend* m_backend;
	QComboBox* m_source;
	QLineEdit* m_base;
	QLineEdit* m_repo;
	QComboBox* m_token;
	QCheckBox* m_onStart;
	QLabel* m_current;
	QLabel* m_found;
	QLabel* m_state;
	QPlainTextEdit* m_notes;
	QPushButton* m_check;
	QPushButton* m_install;
	QPushButton* m_later;
	QPushButton* m_cancel;
	QPushButton* m_restart;
	ActionBar* m_bar;
	QString m_program;
};

/// Rückfragen beim Verbinden eines Ziels, bevor eine Sitzung besteht
/// (Auswahl des Zugangsdatensatzes, Host-Schlüssel eines Sprungservers,
/// Zertifikat eines Hypervisors). Schließen lehnt ab.
class ConnectPromptTab : public ToolTab
{
	Q_OBJECT
public:
	ConnectPromptTab(RjBackend* backend, const QString& targetId, const QString& title, QWidget* parent = nullptr);
	QString targetId() const { return m_target; }
	QString title() const { return m_title; }
	ActionBar* bar() const { return m_bar; }
	SecurityBar* security() const { return m_security; }
	void setMessage(const QString& text);
	/// Offene Rückfragen ablehnen.
	void wipeSecrets() override;

private:
	QString m_target;
	QString m_title;
	QLabel* m_message;
	ActionBar* m_bar;
	SecurityBar* m_security;
};

/// Vorschau eines Imports mit „Übernehmen“ und „Abbrechen“.
class ImportTab : public ToolTab
{
	Q_OBJECT
public:
	ImportTab(RjBackend* backend, const QString& path, const QString& summary, QWidget* parent = nullptr);
};

/// Patchday: Zustand je Ziel, Fortschritt, Protokoll, Abbruch, Ausführen
/// erst nach Bestätigung in der Leiste.
class PatchdayTab : public ToolTab
{
	Q_OBJECT
public:
	PatchdayTab(RjBackend* backend, const QStringList& ids, QWidget* parent = nullptr);
	void refresh();
	/// Längste Dauer eines Neuzeichnens (für die Messung).
	qint64 maxRefreshMs = 0;
	int refreshes = 0;

private:
	void cancelRow(int r);
	RjBackend* m_backend;
	QStringList m_ids;
	QTableWidget* m_table;
	QPlainTextEdit* m_log;
	ActionBar* m_bar;
	QTimer* m_timer;
	qint64 m_version = 0;
};

/// Mehrere Ziele auf einmal anlegen: ein Host pro Zeile, Namensmuster,
/// Vorlage, Zugangsdaten, Vorschau mit Hinweisen je Zeile.
class BulkAddTab : public ToolTab
{
	Q_OBJECT
public:
	BulkAddTab(RjBackend* backend, const QString& parentId, QWidget* parent = nullptr);
	QString parentId() const { return m_parent; }
	void wipeSecrets() override;
	QPlainTextEdit* hosts() const { return m_hosts; }
	/// Vorschau sofort neu berechnen (sonst verzögert beim Tippen).
	void refresh();

private:
	QJsonObject command(const QString& op) const;
	void fillTemplates();
	void updateCredentialMode();
	void create();
	QString edit(const QJsonObject& cmd) const;

	RjBackend* m_backend;
	QString m_parent;
	QComboBox* m_kind;
	QPlainTextEdit* m_hosts;
	QLineEdit* m_pattern;
	QLabel* m_patternError;
	QComboBox* m_template;
	QButtonGroup* m_credMode;
	QWidget* m_directBox;
	QLineEdit* m_user;
	QLineEdit* m_password;
	QWidget* m_entryBox;
	QComboBox* m_entry;
	QWidget* m_namedBox;
	QComboBox* m_named;
	QTableWidget* m_preview;
	QLabel* m_summary;
	QPushButton* m_create;
	QTimer* m_timer;
};

/// Editor für ein Makro: Name und Schritte (Senden, Warten auf; Auswahlschritte als JSON).
class MacroTab : public QWidget
{
	Q_OBJECT
public:
	MacroTab(RjBackend* backend, const QString& id, QWidget* parent = nullptr);
	QString macroId() const { return m_id; }
	QString title() const;
	bool isDirty() const { return m_dirty; }

signals:
	/// Gespeichert (Schaltfläche „Speichern“): der Tab kann geschlossen werden.
	void saved();
	/// Verworfen: der Tab kann geschlossen werden.
	void discarded();
	void titleChanged();
	/// Name oder Inhalt wurden gespeichert (Baum neu laden).
	void indexChanged();

private:
	void load();
	void refreshTable(int select = -1);
	void showStep(int row);
	void storeStep();
	void addStep(const QString& kind);
	void moveStep(int delta);
	void removeStep();
	void markDirty();
	bool save();
	QString summary(const QJsonObject& step) const;
	RjBackend* m_backend;
	QString m_id;
	QString m_origName;
	QJsonArray m_steps;
	bool m_dirty = false;
	bool m_loading = false;
	QLineEdit* m_name;
	QTableWidget* m_table;
	QStackedWidget* m_stack;
	QLineEdit* m_sendText;
	QCheckBox* m_sendEnter;
	QComboBox* m_sendInsert;
	QComboBox* m_waitKind;
	QLineEdit* m_waitPattern;
	QSpinBox* m_waitTimeout;
	QPlainTextEdit* m_waitErrors;
	QPlainTextEdit* m_json;
	QLabel* m_jsonError;
	QLabel* m_linked;
	QLabel* m_status;
	QLabel* m_dirtyLabel;
};

/// Einstellungen des SSH-Terminals: Schrift, Größe und Farben. Änderungen gelten sofort
/// für alle offenen und neuen Terminals und werden gespeichert.
class TerminalSettingsTab : public QWidget
{
	Q_OBJECT
public:
	explicit TerminalSettingsTab(RjBackend* backend, QWidget* parent = nullptr);

private:
	void send(const QJsonObject& change);
	void show(const QJsonObject& settings);
	RjBackend* m_backend;
	QFontComboBox* m_font;
	QCheckBox* m_system;
	QSpinBox* m_size;
	QComboBox* m_scheme;
	QLineEdit* m_hex[4];
	QLabel* m_swatch[4];
	QLabel* m_custom;
	QLabel* m_message;
	QWidget* m_preview;
	QJsonArray m_schemes;
	bool m_updating = false;
};

/// Abdeckung des Hauptfensters zum Entsperren der KDBX.
class UnlockOverlay : public QWidget
{
	Q_OBJECT
public:
	explicit UnlockOverlay(RjBackend* backend, QWidget* parent);
	/// `locked`: Sperre nach Leerlauf oder von Hand – kein Abbrechen möglich.
	void open(const QString& path, bool locked);
	/// Neue KDBX anlegen (Datei, Hauptkennwort mit Wiederholung, optional Schlüsseldatei).
	void openNew();
	void setBusy(bool busy, const QString& message = QString());

signals:
	void unlockRequested(const QString& path, const QString& password, const QString& keyFile);
	void createRequested(const QString& path, const QString& password, const QString& keyFile, bool makeKeyFile);

protected:
	void paintEvent(QPaintEvent*) override;
	bool eventFilter(QObject*, QEvent*) override;

private:
	void submit();
	void setMode(bool create);
	void browse();
	void updateStrength();
	RjBackend* m_backend;
	bool m_create = false;
	bool m_locked = false;
	QLabel* m_title;
	QLabel* m_hint;
	QLineEdit* m_path;
	QLineEdit* m_password;
	QLineEdit* m_password2;
	QLineEdit* m_keyFile;
	QCheckBox* m_makeKey;
	QLabel* m_strength;
	QWidget* m_repeatRow;
	QWidget* m_makeKeyRow;
	QWidget* m_strengthRow;
	QFormLayout* m_form;
	QPushButton* m_browse;
	QPushButton* m_switch;
	QPushButton* m_ok;
	QPushButton* m_cancel;
	QLabel* m_message;
};
