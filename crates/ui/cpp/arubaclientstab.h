#pragma once

// Aruba-Werkzeuge: alle Anwender eines Clusters („Nutzer anzeigen“, `show clients` am VC) mit IP, MAC, SSID, AP,
// Signal und Geschwindigkeit; das Suchfeld filtert nach Anmeldename, IP, MAC oder jedem anderen Feld. Ein Anwender
// taucht beim Roaming auch an mehreren APs auf. Ein Klick auf den AP öffnet dessen Seite. Für den gewählten Anwender
// zeigen drei Knöpfe (auch im Kontextmenü) Anmeldeprobleme, System- und Firewallereignisse an seinem AP. Nur Anzeige.

#include "editor.h"

#include <QtCore/QJsonObject>

class QLabel;
class QLineEdit;
class QPushButton;
class QTableWidget;
class RjBackend;
class UserDiag;

class ArubaClientsTab : public ToolTab
{
	Q_OBJECT
public:
	/// `subject`: der VC (wie in der Übersicht, mit `vc_name`); `where`: Name für die Anzeige.
	ArubaClientsTab(RjBackend* backend, const QJsonObject& subject, const QString& where, QWidget* parent = nullptr);
	/// Kennung für die Ergebnisse (`aruba_clients`).
	QString tag() const { return m_tag; }
	/// VC dieses Tabs (ein Tab je VC).
	QString vcId() const { return m_subject.value(QStringLiteral("id")).toString(); }
	QString title() const;
	void handleResult(const QJsonObject& r);
	/// Liste neu abfragen (der Suchbegriff bleibt).
	void reload();

signals:
	/// Klick auf den AP eines Anwenders (`ap` wie aus der Cluster-Abfrage).
	void apRequested(const QJsonObject& subject, const QJsonObject& ap);
	/// Log am AP des Anwenders (`kind`: auth, system oder security; `filter`: dessen MAC bzw. IP).
	void logRequested(const QJsonObject& subject, const QString& ip, const QString& where, const QString& kind, const QString& filter);

private:
	void applyFilter();
	/// AP des Anwenders in Zeile `row` (wie aus der Cluster-Abfrage); leer, wenn unbekannt.
	QJsonObject apOf(int row) const;
	RjBackend* m_backend;
	QJsonObject m_subject;
	QString m_where;
	QString m_tag;
	QJsonObject m_aps;
	int m_apCol = -1;
	UserDiag* m_diag = nullptr;
	QLineEdit* m_filter;
	QPushButton* m_reload;
	QLabel* m_state;
	QLabel* m_count;
	QTableWidget* m_table;
};
