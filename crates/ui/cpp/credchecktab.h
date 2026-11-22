#pragma once

// Zugangsdaten prüfen (Kontextmenü „Zugangsdaten prüfen …“): je Ziel und Zugangsweg nur die Anmeldung, mehrere
// gleichzeitig; gleiche Zugangsdaten nacheinander und nach einer Ablehnung nicht weiter (Kontosperre). Übersicht mit
// Ergebnis, Filter „nur Fehler“, „Fehlgeschlagene wiederholen“ und „Zugangsdaten bearbeiten“.

#include "editor.h"

#include <QtCore/QStringList>

class QCheckBox;
class QLabel;
class QPushButton;
class QTimer;
class QTreeWidget;
class RjBackend;

class CredCheckTab : public ToolTab
{
	Q_OBJECT
public:
	CredCheckTab(RjBackend* backend, const QStringList& ids, const QString& title, QWidget* parent = nullptr);

signals:
	/// Doppelklick auf eine Zeile: Ziel im Baum zeigen.
	void showTarget(const QString& id);
	/// „Zugangsdaten bearbeiten“: Eigenschaften des Ziels öffnen.
	void editTarget(const QString& id);

private:
	void start(const QStringList& ids);
	void refresh();
	void applyFilter();
	QString edit(const QJsonObject& cmd) const;
	RjBackend* m_backend;
	QString m_title;
	QLabel* m_state;
	QPushButton* m_cancel;
	QPushButton* m_retry;
	QPushButton* m_edit;
	QCheckBox* m_onlyErrors;
	QTreeWidget* m_list;
	QTimer* m_poll;
	QStringList m_failed;
	bool m_running = false;
};
