#pragma once

// Aruba-Werkzeuge: Log eines Controllers oder APs anzeigen (show log system bzw. show log security, wahlweise mit
// „| include <Filter>“). Nur Anzeige; nichts wird gespeichert. Der Filter lässt sich im Tab ändern und neu laden.

#include "editor.h"

#include <QtCore/QJsonObject>

#include <QtCore/QList>
#include <QtCore/QObject>

#include <functional>

class QHBoxLayout;
class QLabel;
class QLineEdit;
class QMenu;
class QTableWidget;
class QPlainTextEdit;
class QPushButton;
class RjBackend;

class ArubaLogTab : public ToolTab
{
	Q_OBJECT
public:
	/// `subject`: der VC (wie in der Übersicht, mit `vc_name`); `ip`: AP-Adresse oder leer (am VC);
	/// `where`: Name für die Anzeige; `kind`: „system“, „security“ oder „auth“.
	ArubaLogTab(RjBackend* backend, const QJsonObject& subject, const QString& ip, const QString& where, const QString& kind, const QString& filter, QWidget* parent = nullptr);
	/// Kennung für die Ergebnisse (`aruba_log`).
	QString tag() const { return m_tag; }
	QString title() const;
	void handleResult(const QJsonObject& r);
	/// Fragt den Filterbegriff ab; false bei Abbruch.
	static bool askFilter(RjBackend* backend, QWidget* parent, const QString& kind, QString* filter);

private:
	void load();
	RjBackend* m_backend;
	QJsonObject m_subject;
	QString m_ip;
	QString m_where;
	QString m_kind;
	QString m_tag;
	QLineEdit* m_filter;
	QPushButton* m_reload;
	QLabel* m_state;
	QPlainTextEdit* m_text;
};

/// Diagnosen für den gewählten Anwender einer `show clients`-Tabelle, an seinem AP: Anmeldeprobleme
/// (auth-trace-buf) und Systemereignisse nach seiner MAC, Firewallereignisse nach seiner IP. Liefert die Knopfzeile
/// und Einträge für das Kontextmenü der Tabelle (siehe `rjui::makeCopyable`).
class UserDiag : public QObject
{
	Q_OBJECT
public:
	/// `apOf`: AP des Anwenders in einer Zeile (`ip`, `name`); leer, wenn unbekannt.
	UserDiag(RjBackend* backend, QTableWidget* table, const std::function<QJsonObject(int row)>& apOf, QWidget* parent);
	/// Zeile „Am AP des gewählten Anwenders:“ mit den drei Knöpfen.
	QHBoxLayout* buttons() const { return m_row; }
	/// Einträge für das Kontextmenü (Zeile `row`).
	void addTo(QMenu* menu, int row);
	/// Knöpfe nach Auswahl, Filter oder neuen Daten freigeben.
	void update();

signals:
	void requested(const QString& ip, const QString& where, const QString& kind, const QString& filter);

private:
	/// Filterbegriff für `kind` in Zeile `row` (MAC bzw. IP des Anwenders); leer, wenn es keinen gibt.
	QString filterOf(int row, const QString& kind) const;
	void request(int row, const QString& kind);
	RjBackend* m_backend;
	QTableWidget* m_table;
	std::function<QJsonObject(int)> m_apOf;
	QHBoxLayout* m_row;
	QList<QPushButton*> m_buttons;
};
