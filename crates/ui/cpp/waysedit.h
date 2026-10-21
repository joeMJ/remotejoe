#pragma once

// Einstellungen eines Ziels → Zugangswege: weitere Wege neben dem Hauptweg (Protokoll, Adresse bzw. URL, Port,
// Zugangsdaten) und der Weg für den Doppelklick.

#include <QtCore/QJsonArray>
#include <QtCore/QJsonObject>
#include <QtCore/QList>
#include <QtWidgets/QWidget>

class QComboBox;
class QLabel;
class QLineEdit;
class QPushButton;
class QVBoxLayout;
class RjBackend;

class WaysEditor : public QWidget
{
	Q_OBJECT
public:
	/// `data`: Antwort von „editor“ (ways, open_with, main_way, way_logins, id).
	WaysEditor(RjBackend* backend, const QJsonObject& data, QWidget* parent = nullptr);
	QJsonArray ways() const;
	/// „main“ oder die ID des Wegs für den Doppelklick.
	QString openWith() const;
	/// Kurzfassung für die Bereichsliste.
	QString summary() const;
	/// Eingegebene Kennwörter der Wege leeren.
	void wipeSecrets();

signals:
	void changed();

private:
	struct Row
	{
		QWidget* box;
		QString id;
		QLineEdit* name;
		QComboBox* protocol;
		QLineEdit* host;
		QLineEdit* port;
		QComboBox* credential;
		/// Eigene Zugangsdaten des Wegs (nur sichtbar bei „Eigene Zugangsdaten“).
		QWidget* ownBox;
		QLineEdit* user;
		QLineEdit* pass;
		/// Protokoll-Optionen des Wegs (aufklappbar); Felder tragen ihren Schlüssel in der Eigenschaft „optkey“.
		QPushButton* optToggle;
		QWidget* optBox;
	};
	/// Felder der Optionen für das gewählte Protokoll neu aufbauen (`o`: bisherige Werte).
	void buildOptions(Row& r, const QJsonObject& o);
	void addRow(const QJsonObject& w);
	void removeRow(QWidget* box);
	void refreshOpenWith();
	void suggest();
	RjBackend* m_backend;
	QString m_target;
	QString m_mainLabel;
	QJsonArray m_logins;
	QVBoxLayout* m_rows;
	QComboBox* m_openWith;
	QLabel* m_hint;
	QList<Row> m_list;
	bool m_loading = false;
};
