#pragma once

// Aruba-Werkzeuge: Log eines Controllers oder APs anzeigen (show log system bzw. show log security, wahlweise mit
// „| include <Filter>“). Nur Anzeige; nichts wird gespeichert. Der Filter lässt sich im Tab ändern und neu laden.

#include "editor.h"

#include <QtCore/QJsonObject>

class QLabel;
class QLineEdit;
class QPlainTextEdit;
class QPushButton;
class RjBackend;

class ArubaLogTab : public ToolTab
{
	Q_OBJECT
public:
	/// `subject`: der VC (wie in der Übersicht, mit `vc_name`); `ip`: AP-Adresse oder leer (am VC);
	/// `where`: Name für die Anzeige; `kind`: „system“ oder „security“.
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
