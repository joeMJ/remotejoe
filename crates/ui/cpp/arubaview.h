#pragma once

// Aruba-Werkzeuge (Einstellungen → Zusatzfunktionen): Cluster-Karte in der Übersicht eines virtuellen Controllers.
// Zähler (im Cluster, online, offline, Anwender), darunter je AP eine Kachel „.Oktett | Name/MAC“ mit der Zahl der
// angemeldeten Anwender; der Conductor trägt einen Stern, APs außerhalb des Segments zeigen die volle IP.
// Ein Klick auf einen erreichbaren AP meldet `apClicked`.

#include <QtCore/QJsonObject>
#include <QtCore/QStringList>
#include <QtWidgets/QWidget>

class QButtonGroup;
class QLabel;
class QPushButton;
class RjBackend;
namespace rjui
{
class FlowBox;
}

class ArubaClusterView : public QWidget
{
	Q_OBJECT
public:
	ArubaClusterView(RjBackend* backend, QWidget* parent = nullptr);
	/// Ergebnis von `aruba_cluster` (oder leer: noch nicht abgefragt).
	void setCluster(const QJsonObject& c);
	/// Name des VC im Baum (für „Offline-MAC-Adressen kopieren“).
	void setName(const QString& name);
	/// MACs der offline-APs eines Clusters.
	static QStringList offlineMacs(const QJsonObject& c);
	/// Text für „Offline-MAC-Adressen kopieren“: Cluster, Segment, VC-IP, dann die MACs untereinander.
	static QString offlineText(RjBackend* backend, const QString& name, const QJsonObject& c);
	/// Zahl der offline-APs, ggf. mit gemerkter Zahl („9 (zuvor 12)“); `tip` erhält den Zeitpunkt des Merkens.
	static QString offlineCount(RjBackend* backend, const QJsonObject& c, QString* tip = nullptr);
	/// Letzter Neustart des Conductors („Letzter Neustart: … – …“); `tip` erhält die Originalzeile.
	/// `label`: mit „Letzter Neustart“ davor (Karte), sonst nur Zeitpunkt und Grund (Tabellenzelle).
	static QString rebootText(RjBackend* backend, const QJsonObject& c, QString* tip = nullptr, bool label = true);
	/// Abfrage läuft (Knopf gesperrt, Hinweis).
	void setBusy(bool busy);
	void setError(const QString& text);

signals:
	void queryRequested();
	/// Werkzeug für diesen Controller: „analyse“, „inventar“, „reload“ oder „update“.
	void toolRequested(const QString& tool);
	/// Log am Controller anzeigen: `kind` „system“ oder „security“, `filtered` mit Filterbegriff.
	void logRequested(const QString& kind, bool filtered);
	/// Anwender des Clusters anzeigen.
	void usersRequested();
	/// Clusteraufbrüche suchen (Segment auf 4343).
	void breakoutsRequested();
	/// „Anzahl Offline-APs merken“.
	void markRequested();
	/// „Offline-APs löschen“: alle offline-MACs dieses Clusters.
	void offlineDeleteRequested(const QStringList& macs);
	/// Kachel eines online-APs angeklickt (`name`, `ip`, `serial`, …).
	void apClicked(const QJsonObject& ap);

private:
	void render();
	RjBackend* m_backend;
	QJsonObject m_cluster;
	QLabel* m_head;
	QLabel* m_managed;
	QPushButton* m_query;
	QPushButton* m_reload = nullptr;
	QPushButton* m_update = nullptr;
	QLabel* m_total;
	QLabel* m_online;
	QLabel* m_offline;
	class QFrame* m_offlineTile;
	QString m_name;
	QLabel* m_clients;
	QButtonGroup* m_filter;
	rjui::FlowBox* m_box;
	QLabel* m_foot;
};
