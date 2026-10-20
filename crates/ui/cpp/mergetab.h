#pragma once

// „Gleiche Adressen zusammenfassen“ (Kontextmenü): zeigt die Ziele der Auswahl mit gleicher Adresse als
// Gruppen. Je Gruppe wählt der Nutzer, ob und welche Ziele zusammengefasst werden, das Hauptziel und dessen
// Namen; die übrigen Ziele werden Zugangswege des Hauptziels. Ordner bleiben unberührt; ein Schritt
// „Rückgängig“ stellt alles wieder her.

#include "editor.h"

#include <QtCore/QStringList>

class QComboBox;
class QLabel;
class QLineEdit;
class QPushButton;
class QTreeWidget;
class QTreeWidgetItem;
class RjBackend;

class MergeTab : public ToolTab
{
	Q_OBJECT
public:
	MergeTab(RjBackend* backend, const QStringList& ids, const QString& title, QWidget* parent = nullptr);

	/// Der Index hat sich geändert (auch von außen, z. B. Bearbeiten → Rückgängig): Gruppen neu laden.
	void refresh();

signals:
	/// Doppelklick auf ein Ziel: im Baum zeigen.
	void showTarget(const QString& id);

private:
	void load();
	void select(QTreeWidgetItem* group);
	void check(QTreeWidgetItem* group);
	void updateRoles(QTreeWidgetItem* group);
	void updateState();
	void apply();
	void undo();
	QStringList others(QTreeWidgetItem* group) const;
	QString edit(const QJsonObject& cmd) const;
	RjBackend* m_backend;
	QStringList m_ids;
	QLabel* m_state;
	QPushButton* m_apply;
	QPushButton* m_undo;
	QTreeWidget* m_list;
	QLabel* m_groupHead;
	QComboBox* m_keep;
	QLineEdit* m_name;
	QLabel* m_notes;
	QTreeWidgetItem* m_current = nullptr;
	bool m_filling = false;
	QString m_done;
};
