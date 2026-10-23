#pragma once

// „Gleiche Zugangsdaten zusammenführen“ (Extras bzw. Kontextmenü für Ordner und Auswahl): direkt hinterlegte
// Zugangsdaten mit gleichem Benutzer und Kennwort an mehreren Stellen; je Gruppe ein Anmeldeobjekt anlegen oder ein
// vorhandenes mit denselben Daten verwenden. Zeigt nie Kennwörter. Rückgängig über Bearbeiten → Rückgängig.

#include "editor.h"

#include <QtCore/QJsonArray>
#include <QtCore/QStringList>

class QLabel;
class QPushButton;
class QTreeWidget;
class RjBackend;

class CredMergeTab : public ToolTab
{
	Q_OBJECT
public:
	/// `scope`: nur unter diesen Zielen bzw. Ordnern (leer: ganzer Baum).
	CredMergeTab(RjBackend* backend, const QStringList& scope, QWidget* parent = nullptr);
	void load();

signals:
	/// Doppelklick auf eine Stelle: im Baum zeigen.
	void showTarget(const QString& id);

private:
	void apply(const QJsonArray& entries, bool create, const QString& suggestion);
	QJsonObject call(const QJsonObject& cmd) const;
	RjBackend* m_backend;
	QStringList m_scope;
	QLabel* m_state;
	QTreeWidget* m_list;
	QPushButton* m_merge;
};
