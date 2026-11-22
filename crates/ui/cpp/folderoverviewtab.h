#pragma once

// Übersicht eines Ordners (Einfachklick auf einen Ordner im Baum): die Elemente direkt im Ordner als Tabelle mit Typ
// (Symbol wie im Baum), Name, Adresse und den Werten aus dem gespeicherten Inventar (Version, System, CPUs, Speicher,
// Laufzeit, Seriennummer, MAC, Stand). Unterordner stehen am Ende, nur mit Symbol und Titel. Abgefragt wird nichts.
// Einfachklick öffnet die Übersicht des Elements, Doppelklick verbindet wie im Baum.

#include "editor.h"

#include <QtCore/QList>
#include <QtGui/QIcon>

class QLabel;
class QLineEdit;
class QTableWidget;
class RjBackend;

class FolderOverviewTab : public ToolTab
{
	Q_OBJECT
public:
	/// Ein Element des Ordners, wie es im Baum steht.
	struct Item
	{
		QString id;
		QString name;
		QString type;
		QString host;
		QIcon icon;
		bool folder = false;
	};
	explicit FolderOverviewTab(RjBackend* backend, QWidget* parent = nullptr);
	/// Ordner zeigen: `path` für die Kopfzeile, `items` in Baumreihenfolge (Unterordner kommen ans Ende).
	void showFolder(const QString& id, const QString& name, const QString& path, const QList<Item>& items);
	QString folderId() const { return m_id; }
	QString title() const;

signals:
	void targetClicked(const QString& id, const QIcon& icon);
	void targetActivated(const QString& id);
	void folderClicked(const QString& id);

private:
	void applyFilter();
	RjBackend* m_backend;
	QString m_id;
	QString m_name;
	QLabel* m_caption;
	QLabel* m_sub;
	QLineEdit* m_filter;
	QLabel* m_count;
	QTableWidget* m_table;
};
