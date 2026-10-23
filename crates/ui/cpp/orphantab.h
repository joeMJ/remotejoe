#pragma once

// „Verwaiste KDBX-Einträge prüfen“ (Extras): eigene Einträge der Gruppe `remotejoe`, auf die nichts mehr verweist.
// Markierte wandern in den Papierkorb der KDBX (nie endgültig gelöscht); „Rückgängig“ holt sie zurück. Einträge
// außerhalb der Gruppe `remotejoe` werden nie gezeigt oder angefasst. Zeigt nie Kennwörter.

#include "editor.h"

#include <QtCore/QJsonArray>

class QLabel;
class QPushButton;
class QTableWidget;
class RjBackend;

class OrphanTab : public ToolTab
{
	Q_OBJECT
public:
	explicit OrphanTab(RjBackend* backend, QWidget* parent = nullptr);
	void load();

private:
	void move(bool recycle, const QJsonArray& entries);
	QJsonObject call(const QJsonObject& cmd) const;
	RjBackend* m_backend;
	QLabel* m_state;
	QTableWidget* m_table;
	QPushButton* m_recycle;
	QPushButton* m_undo;
	QJsonArray m_last;
};
