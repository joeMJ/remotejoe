#pragma once

// Aruba-Werkzeuge: Clusteraufbrüche suchen. Ein AP, der die PAPI-Verbindung zum Conductor verliert (falsche Uhrzeit,
// Kabel), fällt aus dem Schwarm, bleibt aber per IP erreichbar und beansprucht selbst die VC-IP. Der Tab prüft das
// Segment des Schwarms auf TCP 4343 (ohne die online-APs und die VC-IP) und listet die Geräte, die antworten.
// „Erkennen“ fragt ein Gerät per SSH mit den Zugangsdaten des VC ab (Name, MAC, beanspruchte VC-IP) und gleicht es
// mit den offline gemeldeten APs ab; „SSH-Sitzung“ und „Web-Oberfläche“ öffnen es. Nur Anzeige.

#include "editor.h"

#include <QtCore/QHash>
#include <QtCore/QJsonObject>
#include <QtCore/QList>

class QLabel;
class QPushButton;
class QTableWidget;
class RjBackend;

class ArubaBreakoutTab : public ToolTab
{
	Q_OBJECT
public:
	/// `subject`: der VC (wie in der Übersicht, mit `vc_name`); `where`: Name für die Anzeige.
	ArubaBreakoutTab(RjBackend* backend, const QJsonObject& subject, const QString& where, QWidget* parent = nullptr);
	/// Kennung für die Ergebnisse (`aruba_breakouts`, `aruba_breakout_id`).
	QString tag() const { return m_tag; }
	/// VC dieses Tabs (ein Tab je VC).
	QString vcId() const { return m_subject.value(QStringLiteral("id")).toString(); }
	QString title() const;
	/// Ergebnis der Suche (Treffer ersetzen die Tabelle).
	void handleResult(const QJsonObject& r);
	/// Ergebnis von „Erkennen“ für einen Treffer.
	void handleIdentify(const QJsonObject& r);
	/// Segment neu absuchen.
	void search();

signals:
	/// Verbindung zum Gerät: `way` = `ssh:<IP>` bzw. `web:<IP>` (Zugangsdaten des VC).
	void connectWayRequested(const QJsonObject& subject, const QString& way);

private:
	void identify(const QString& ip);
	void setRow(int row, const QJsonObject& hit);
	void updateActions();
	QString currentIp() const;
	RjBackend* m_backend;
	QJsonObject m_subject;
	QString m_where;
	QString m_tag;
	/// Laufende „Erkennen“-Abfragen (IP).
	QHash<QString, bool> m_identifying;
	QPushButton* m_again;
	QLabel* m_result;
	QLabel* m_state;
	QLabel* m_offline;
	QTableWidget* m_table;
	QList<QPushButton*> m_rowButtons;
	QPushButton* m_all;
};
