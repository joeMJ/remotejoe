#pragma once

// Aruba-Werkzeuge (Einstellungen → Zusatzfunktionen): Cluster-Karte in der Übersicht eines virtuellen Controllers.
// Zähler (im Cluster, online, offline, Anwender), darunter je AP eine Kachel „.Oktett | Name/MAC“ mit der Zahl der
// angemeldeten Anwender; der Conductor trägt einen Stern, APs außerhalb des Segments zeigen die volle IP.
// Ein Klick auf einen erreichbaren AP meldet `apClicked`.

#include <QtCore/QJsonObject>
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
	/// Abfrage läuft (Knopf gesperrt, Hinweis).
	void setBusy(bool busy);
	void setError(const QString& text);

signals:
	void queryRequested();
	/// „ap_check …“: Aruba-Werkzeuge für diesen Controller.
	void toolsRequested();
	/// Kachel eines online-APs angeklickt (`name`, `ip`, `serial`, …).
	void apClicked(const QJsonObject& ap);

private:
	void render();
	RjBackend* m_backend;
	QJsonObject m_cluster;
	QLabel* m_head;
	QLabel* m_managed;
	QPushButton* m_query;
	QLabel* m_total;
	QLabel* m_online;
	QLabel* m_offline;
	QLabel* m_clients;
	QButtonGroup* m_filter;
	rjui::FlowBox* m_box;
	QLabel* m_foot;
};
