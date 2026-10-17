#pragma once

#include <QtWidgets/QDialog>
#include <QtWidgets/QWidget>

class QLabel;
class QToolButton;
class RjBackend;

/// Eigene Titelleiste: links das Logo, daneben „remotejoe“ und die Version, rechts die Fensterknöpfe.
/// Verschieben und Größe ändern übernimmt das Fenstersystem (`startSystemMove`, `startSystemResize`).
class TitleBar : public QWidget
{
	Q_OBJECT
public:
	TitleBar(QWidget* window, const QString& name, const QString& version, QWidget* parent = nullptr);
	/// Symbol des Maximieren-Knopfs und Tooltip dem Fensterzustand anpassen.
	void updateState();

protected:
	void mousePressEvent(QMouseEvent* e) override;
	void mouseDoubleClickEvent(QMouseEvent* e) override;

private:
	void toggleMaximized();
	QWidget* m_window;
	QToolButton* m_max;
};

/// „Über remotejoe“: Logo, Version, Lizenz und Links (das einzige Popup des Programms).
class AboutDialog : public QDialog
{
	Q_OBJECT
public:
	AboutDialog(RjBackend* backend, const QString& version, QWidget* parent = nullptr);
};

/// Adresse der README auf GitHub (Hilfe → Hilfe).
extern const char* const kHelpUrl;
