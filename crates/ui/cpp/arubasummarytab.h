#pragma once

// Aruba-Werkzeuge: Clusterübersicht für mehrere VCs (Rechtsklick → Aruba → „Cluster abfragen“ bzw.
// „Clusterübersicht“). Eine Zeile je als Aruba-AP erkanntem Ziel der Auswahl (Ordner mit Unterordnern) mit dem
// zuletzt gespeicherten Stand: Verwaltung, Zähler, Offline-APs, letzter Neustart, Zeitpunkt. „Neu abfragen“ fragt alle
// VCs ab, höchstens so viele gleichzeitig wie bei „Alle verbinden“. Rechtsklick: Offline-MACs kopieren, Anzahl
// Offline-APs merken (für alle markierten); Doppelklick öffnet die Übersicht des VC.

#include "editor.h"

#include <QtCore/QHash>
#include <QtCore/QJsonObject>
#include <QtCore/QStringList>

class QLabel;
class QPushButton;
class QTableWidget;
class RjBackend;

class ArubaSummaryTab : public ToolTab
{
	Q_OBJECT
public:
	/// `ids`: gewählte Ziele bzw. Ordner; `where`: Name für die Anzeige; `query`: gleich alle abfragen.
	ArubaSummaryTab(RjBackend* backend, const QStringList& ids, const QString& where, bool query, QWidget* parent = nullptr);
	/// Auswahl dieses Tabs (ein Tab je Auswahl).
	QString selection() const { return m_ids.join(QLatin1Char(',')); }
	QString title() const;
	/// Ergebnis von „Cluster abfragen“ für ein Ziel (Art `ov_aruba_cluster`).
	void handleResult(const QString& id, const QJsonObject& r);
	/// Alle VCs neu abfragen.
	void queryAll();

signals:
	/// Doppelklick: Übersicht des VC öffnen.
	void openTarget(const QString& id);

private:
	void load();
	void startNext();
	void fillRow(int row);
	void updateState();
	int rowOf(const QString& id) const;
	QStringList selectedIds() const;
	RjBackend* m_backend;
	QStringList m_ids;
	QString m_where;
	/// Je Zeile: Ziel-ID, Name und zuletzt bekannter Cluster (oder Fehler).
	struct Row
	{
		QString id;
		QString name;
		QJsonObject cluster;
		QString error;
		bool waiting = false;
		bool running = false;
	};
	QList<Row> m_rows;
	int m_parallel = 4;
	QLabel* m_state;
	QPushButton* m_query;
	QPushButton* m_mark;
	QTableWidget* m_table;
};
