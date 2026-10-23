#pragma once

// Claude-Anbindung: lokaler Kanal für `rj-mcp` (Unix-Socket bzw. Named Pipe, nur das eigene Benutzerkonto). Je
// Verbindung eine Anfrage als JSON-Zeile, eine Antwortzeile zurück. Aktionen fragen vorher nach (bis 2 min, dann
// abgelehnt) und werden nach der Freigabe vom Hauptfenster ausgeführt. Der Name des Kanals steht in `mcp-kanal` im
// Datenordner.

#include <QtCore/QJsonObject>
#include <QtCore/QObject>

class QLocalServer;
class QLocalSocket;
class QWidget;
class RjBackend;

class McpServer : public QObject
{
	Q_OBJECT
public:
	McpServer(RjBackend* backend, QWidget* window);
	~McpServer() override;

signals:
	/// Freigegebene Aktion ausführen (`verbinden`, `inventar_abfragen`, `cluster_abfragen`, `nutzer_anzeigen`).
	void actionApproved(const QString& action, const QString& targetId);

private:
	void handle(QLocalSocket* s, const QJsonObject& request);
	void reply(QLocalSocket* s, const QJsonObject& answer);
	QJsonObject call(const QJsonObject& cmd) const;
	RjBackend* m_backend;
	QWidget* m_window;
	QLocalServer* m_server;
	QString m_file;
};
