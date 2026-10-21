#pragma once

// „Geteilte Zugangsdaten prüfen“ (Extras): eigene KDBX-Einträge, die direkt an mehreren Zielen, Ordnern oder Wegen
// eingetragen sind (entsteht z. B. beim Duplizieren mit älteren Versionen). Je Eintrag „Trennen“ (eigene Kopie je
// weiterer Stelle) oder „In Anmeldeobjekt umwandeln“ (gewolltes Teilen). Zeigt nie Kennwörter.

#include "editor.h"

class QLabel;
class QPushButton;
class QTreeWidget;
class RjBackend;

class SharedCredTab : public ToolTab
{
	Q_OBJECT
public:
	explicit SharedCredTab(RjBackend* backend, QWidget* parent = nullptr);
	void load();

signals:
	/// Doppelklick auf eine Stelle: im Baum zeigen.
	void showTarget(const QString& id);

private:
	void act(const char* op);
	QString edit(const QJsonObject& cmd) const;
	RjBackend* m_backend;
	QLabel* m_state;
	QTreeWidget* m_list;
	QPushButton* m_split;
	QPushButton* m_login;
};
