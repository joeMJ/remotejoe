#pragma once

#include "editor.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonObject>
#include <QtCore/QList>

class QGridLayout;
class QLabel;
class QLineEdit;
class QPlainTextEdit;
class QPushButton;
class QScrollArea;
class QTimer;
class QWidget;
class RjBackend;

/// Objektübersicht beim Einfachklick auf ein Ziel: Zielname, angebotene Dienste (grüner Punkt bei
/// Erreichbarkeit), Inventar (je Feld Quelle und Datum, von Hand korrigierbar, Abweichungen zum
/// Übernehmen), Notizen und Prüfungen (Ping, Traceroute). Es gibt nur einen solchen Tab; ein Klick
/// auf ein anderes Ziel zeigt dieses im selben Tab.
class OverviewTab : public ToolTab
{
	Q_OBJECT
public:
	OverviewTab(RjBackend* backend, QWidget* parent = nullptr);
	/// Ziel (`id`) oder Gast (`hv`, `guest`, `name`, `state`, `address`) anzeigen; lädt die Daten und startet
	/// die Prüfung der Dienste.
	void showSubject(const QJsonObject& subject);
	QString targetName() const { return m_name; }
	/// Ergebnis einer Hintergrundaufgabe (`ov_probe`, `ov_query`).
	void handleResult(const QString& kind, const QString& target, const QJsonObject& r);

signals:
	void titleChanged();

private:
	QJsonObject call(const QJsonObject& cmd) const;
	void load();
	void probe(bool force);
	void query();
	void startCheck(const QString& kind);
	void pollRun();
	void saveNotes();
	void showQueryError(const QJsonObject& r);
	void renderServices(const QJsonValue& s);
	void renderInventory(const QJsonArray& fields);
	QString fieldLabel(const QString& key) const;

	RjBackend* m_backend;
	QJsonObject m_subject;
	/// Schlüssel des Angezeigten (Ziel-ID oder Gast), wie ihn die Ergebnisse tragen.
	QString m_key;
	QString m_name;
	QLabel* m_notesHead;
	bool m_notesEditable = false;
	QLabel* m_title;
	QLabel* m_sub;
	QLabel* m_services;
	QPushButton* m_servicesAgain;
	QWidget* m_inventoryBox;
	QGridLayout* m_inventory;
	QPushButton* m_query;
	QLabel* m_queryState;
	QPlainTextEdit* m_notes;
	QPushButton* m_notesSave;
	QLabel* m_notesState;
	QPushButton* m_ping;
	QPushButton* m_trace;
	QPushButton* m_stop;
	QPlainTextEdit* m_output;
	QLabel* m_arubaHead;
	QLabel* m_aruba;
	QTimer* m_poll;
	QScrollArea* m_scroll;
};
