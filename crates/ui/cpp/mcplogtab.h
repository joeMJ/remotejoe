#pragma once

// Extras → „Claude-Protokoll“: alle Anfragen über die Claude-Anbindung dieser Sitzung (Zeit, Werkzeug, Angaben,
// Ergebnis), neueste oben. Wird laufend aufgefrischt.

#include "editor.h"

class QTableWidget;
class QTimer;
class RjBackend;

class McpLogTab : public ToolTab
{
	Q_OBJECT
public:
	explicit McpLogTab(RjBackend* backend, QWidget* parent = nullptr);

private:
	void refresh();
	RjBackend* m_backend;
	QTableWidget* m_table;
	QTimer* m_timer;
	int m_count = -1;
};
