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
	/// `id`: Ziel (VC) oder Ordner; `name` für die Anzeige.
	ApToolsTab(RjBackend* backend, const QString& id, const QString& name, QWidget* parent = nullptr);
	QString id() const { return m_id; }
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
	QLabel* m_collection;
	QLabel* m_python;
	QPushButton* m_setup;
	QPushButton* m_start;
	QPushButton* m_stop;
	QLabel* m_state;
	QPlainTextEdit* m_output;
	QVBoxLayout* m_resultsBox;
	QTimer* m_poll;
	int m_shownResults = -1;
};
