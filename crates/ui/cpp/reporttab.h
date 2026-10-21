#pragma once

// Hilfe → „Fehler melden …“: Art, Titel und Beschreibung erfassen, auf Wunsch die letzten Diagnosezeilen anhängen,
// alles bereinigt (keine Adressen, Rechner-, Benutzer- oder Zielnamen) in einer Vorschau zeigen und im Browser als
// GitHub-Issue öffnen. Abgeschickt wird dort vom Anwender selbst; remotejoe sendet nichts.

#include "editor.h"

class QCheckBox;
class QComboBox;
class QLabel;
class QLineEdit;
class QPlainTextEdit;
class QPushButton;
class RjBackend;

class ReportTab : public ToolTab
{
	Q_OBJECT
public:
	explicit ReportTab(RjBackend* backend, QWidget* parent = nullptr);

private:
	QJsonObject build() const;
	void preview();
	void open();
	RjBackend* m_backend;
	QComboBox* m_kind;
	QLineEdit* m_title;
	QPlainTextEdit* m_text;
	QCheckBox* m_diag;
	QPlainTextEdit* m_preview;
	QLabel* m_state;
	QPushButton* m_open;
};
