#pragma once

// Aruba-Werkzeuge: ap_check aus den Aruba Instant Python Tools für einen Controller (VC) oder alle Controller eines
// Ordners. Zeigt den Stand der Sammlung und der Python-Umgebung (Einrichten/Aktualisieren), die Ausgabe des Laufs
// und die Ergebnisdateien; geöffnet wird mit dem Standardprogramm des Systems.

#include "editor.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonObject>

class QLabel;
class QPlainTextEdit;
class QPushButton;
class QTimer;
class QVBoxLayout;
class RjBackend;

class ApToolsTab : public ToolTab
{
	Q_OBJECT
public:
	/// `id`: Ziel (VC) oder Ordner; `name` für die Anzeige; `tool`: „analyse“ (ap_check) oder „inventar“ (ap_list).
	ApToolsTab(RjBackend* backend, const QString& id, const QString& name, const QString& tool, QWidget* parent = nullptr);
	QString id() const { return m_id; }
	QString tool() const { return m_tool; }
	QString title() const;
	/// Ergebnis von `aptools_setup`.
	void handleSetup(const QJsonObject& r);

private:
	QJsonObject call(const QJsonObject& cmd) const;
	void refreshStatus();
	void setup(bool installPython);
	void start();
	void poll();
	void showResults(const QJsonArray& results);
	RjBackend* m_backend;
	QString m_id;
	QString m_name;
	QString m_tool;
	QLabel* m_collection;
	QLabel* m_python;
	QPushButton* m_setup;
	QPushButton* m_start;
	QPushButton* m_stop;
	QLabel* m_state;
	QPlainTextEdit* m_output;
	QVBoxLayout* m_resultsBox;
	QTimer* m_poll;
	/// Zeigt während des Einrichtens den laufenden Schritt mit Sekunden.
	QTimer* m_setupTimer;
	int m_shownResults = -1;
};

// Schreibende Aruba-Werkzeuge (Cluster neustarten, Cluster aktualisieren): je Aktion ein eigener Tab mit
// Statustafel (Update: Zustand je Cluster), fortlaufendem Protokoll und den Protokolldateien.
class ArubaWriteTab : public ToolTab
{
	Q_OBJECT
public:
	/// Bestätigung vor dem Start: betroffene Controller mit bekanntem Stand; beim Update Firmware-URL und
	/// „danach neu starten“. Liefert false bei Abbruch.
	static bool confirm(RjBackend* backend, QWidget* parent, const QString& id, const QString& name, const QString& tool, QString* url, bool* reboot);
	/// Startet den Lauf und zeigt ihn; `startError` erhält eine Fehlermeldung, wenn er nicht starten konnte.
	ArubaWriteTab(RjBackend* backend, const QString& id, const QString& name, const QString& tool, const QString& url, bool reboot, QWidget* parent = nullptr);
	QString title() const;
	bool started() const { return m_started; }

private:
	QJsonObject call(const QJsonObject& cmd) const;
	void poll();
	RjBackend* m_backend;
	QString m_id;
	QString m_name;
	QString m_tool;
	bool m_started = false;
	QLabel* m_state;
	QPushButton* m_stop;
	QPlainTextEdit* m_board;
	QPlainTextEdit* m_output;
	QVBoxLayout* m_resultsBox;
	QTimer* m_poll;
	int m_shownResults = -1;
};
