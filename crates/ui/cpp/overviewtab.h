#pragma once

#include "editor.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonObject>
#include <QtGui/QIcon>

class QGridLayout;
class QCheckBox;
class QLabel;
class QLineEdit;
class QPlainTextEdit;
class QPushButton;
class QScrollArea;
class QTimer;
class QVBoxLayout;
class QWidget;
class RjBackend;
namespace rjui
{
class FlowLayout;
class FlowBox;
}

/// Objektübersicht beim Einfachklick auf ein Ziel oder einen Gast, im Kartenstil: Kopfkarte (Symbol, Name,
/// Kurzzeile, Verbinden), Kennzahlen als Kacheln (von Hand korrigierbar, neue Werte zum Übernehmen),
/// angebotene Dienste als Plaketten, Notizen, Prüfungen (Ping, Traceroute) und – bei Aruba – eine eigene Karte.
/// Es gibt nur einen solchen Tab; ein Klick auf ein anderes Ziel zeigt dieses im selben Tab.
class OverviewTab : public ToolTab
{
	Q_OBJECT
public:
	OverviewTab(RjBackend* backend, QWidget* parent = nullptr);
	/// Ziel (`id`) oder Gast (`hv`, `guest`, `name`, `state`, `address`) anzeigen; lädt die Daten und startet
	/// die Prüfung der Dienste. `icon`: Symbol wie im Baum.
	void showSubject(const QJsonObject& subject, const QIcon& icon = QIcon());
	QString targetName() const { return m_name; }
	/// Ergebnis einer Hintergrundaufgabe (`ov_probe`, `ov_query`).
	void handleResult(const QString& kind, const QString& target, const QJsonObject& r);

signals:
	void titleChanged();
	/// „Verbinden“ in der Kopfkarte.
	void connectRequested(const QJsonObject& subject);

private:
	QJsonObject call(const QJsonObject& cmd) const;
	void load();
	void applyInventorySwitch(const QJsonObject& r);
	void probe(bool force);
	void query();
	void startCheck(const QString& kind);
	void pollRun();
	void saveNotes();
	void showQueryError(const QJsonObject& r);
	void setQueryState(const QString& html);
	void renderServices(const QJsonValue& s);
	void renderInventory(const QJsonArray& fields);
	void updateSub();
	QString fieldLabel(const QString& key) const;

	RjBackend* m_backend;
	QJsonObject m_subject;
	/// Schlüssel des Angezeigten (Ziel-ID oder Gast), wie ihn die Ergebnisse tragen.
	QString m_key;
	QString m_name;
	QString m_subBase;
	QJsonArray m_inventory;
	bool m_notesEditable = false;
	QScrollArea* m_scroll;
	QLabel* m_crumb;
	QLabel* m_icon;
	QLabel* m_title;
	QLabel* m_sub;
	QPushButton* m_connect;
	QLabel* m_tileValue[4] = {};
	QLabel* m_inventorySource;
	QWidget* m_pendingBox;
	QVBoxLayout* m_pending;
	QPushButton* m_editToggle;
	QWidget* m_inventoryBox;
	QGridLayout* m_inventoryGrid;
	QPushButton* m_query;
	QCheckBox* m_inventoryOn;
	QLabel* m_inventoryRule;
	QLabel* m_queryState;
	QLabel* m_servicesInfo;
	rjui::FlowBox* m_chipsBox;
	rjui::FlowLayout* m_chips;
	QPushButton* m_servicesAgain;
	QWidget* m_notesCard;
	QPlainTextEdit* m_notes;
	QPushButton* m_notesSave;
	QLabel* m_notesState;
	QPushButton* m_ping;
	QPushButton* m_trace;
	QPushButton* m_stop;
	QPlainTextEdit* m_output;
	QWidget* m_arubaCard;
	QTimer* m_poll;
};
