#pragma once

// Masseninventar (Kontextmenü „Inventar für alle Ziele abfragen“): fragt das Inventar aller Ziele der Auswahl
// ab, mehrere gleichzeitig. Kartenstil: Kopfkarte mit Fortschritt und Knöpfen, darunter die Ziele mit Ergebnis.

#include "editor.h"

#include <QtCore/QStringList>

class QLabel;
class QPushButton;
class QTimer;
class QTreeWidget;
class RjBackend;

class MassInventoryTab : public ToolTab
{
	Q_OBJECT
public:
	MassInventoryTab(RjBackend* backend, const QStringList& ids, const QString& title, QWidget* parent = nullptr);

signals:
	/// Doppelklick auf eine Zeile: Ziel im Baum zeigen.
	void showTarget(const QString& id);
	/// Lauf beendet: Gerätearten (Symbole im Baum) können sich geändert haben.
	void finished();

private:
	void start(const QStringList& ids);
	void refresh();
	QString edit(const QJsonObject& cmd) const;
	RjBackend* m_backend;
	QString m_title;
	QLabel* m_head;
	QLabel* m_state;
	QPushButton* m_cancel;
	QPushButton* m_retry;
	QTreeWidget* m_list;
	QTimer* m_poll;
	QStringList m_failed;
	bool m_running = false;
};
