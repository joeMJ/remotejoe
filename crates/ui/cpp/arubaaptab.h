#pragma once

// Aruba-Werkzeuge: Seite eines einzelnen APs eines Clusters (Klick auf seine Kachel in der Cluster-Karte).
// Abgefragt per SSH am AP mit den Zugangsdaten des VC: Seriennummer, MAC, Laufzeit, aktive SSIDs (show networks)
// und angemeldete Anwender (show clients). „SSH-Sitzung“ öffnet eine Sitzung zum AP. Für den gewählten Anwender
// zeigen drei Knöpfe (auch im Kontextmenü) Anmeldeprobleme, System- und Firewallereignisse an diesem AP.

#include "editor.h"

#include <QtCore/QJsonObject>

class QLabel;
class QPushButton;
class QTableWidget;
class RjBackend;
class UserDiag;

class ArubaApTab : public ToolTab
{
	Q_OBJECT
public:
	/// `subject`: der VC (wie in der Übersicht), `ap`: Eintrag aus der Cluster-Abfrage.
	ArubaApTab(RjBackend* backend, const QJsonObject& subject, const QJsonObject& ap, QWidget* parent = nullptr);
	/// `<VC-ID>|<IP>` – Ziel der Ergebnisse von `aruba_ap`.
	QString key() const { return m_key; }
	QString title() const;
	void handleResult(const QJsonObject& r);
	void query();

signals:
	/// SSH-Sitzung zum AP: wie ein Zugangsweg des VC (`way` = `ssh:<IP>`).
	void connectWayRequested(const QJsonObject& subject, const QString& way);
	/// Log an diesem AP anzeigen.
	void logRequested(const QJsonObject& subject, const QString& ip, const QString& where, const QString& kind, bool filtered);
	/// Log für einen Anwender an diesem AP (`filter`: seine MAC bzw. IP), siehe `UserDiag`.
	void userLogRequested(const QJsonObject& subject, const QString& ip, const QString& where, const QString& kind, const QString& filter);

private:
	static void fillTable(QTableWidget* t, const QJsonObject& table);
	RjBackend* m_backend;
	QJsonObject m_subject;
	QJsonObject m_ap;
	QString m_key;
	QLabel* m_sub;
	QLabel* m_state;
	QPushButton* m_again;
	QLabel* m_serial;
	QLabel* m_mac;
	QLabel* m_uptime;
	QLabel* m_system;
	QLabel* m_netsInfo;
	QTableWidget* m_nets;
	QLabel* m_clientsInfo;
	QTableWidget* m_clients;
	UserDiag* m_diag = nullptr;
};
