#pragma once

#include <QtWidgets/QDialog>
#include <QtWidgets/QWidget>

class QLabel;
class QToolButton;
class RjBackend;

class QMenu;

/// Logo, Name und Version; zieht das Fenster (Doppelklick maximiert).
class BrandStrip : public QWidget
{
	Q_OBJECT
public:
	BrandStrip(QWidget* window, const QString& name, const QString& version, QWidget* parent = nullptr);
	/// Die Tab-Leiste legt Eck-Widgets nach `sizeHint` an; bei festgesetzter Höhe gilt diese.
	QSize sizeHint() const override;

protected:
	void mousePressEvent(QMouseEvent* e) override;
	void mouseDoubleClickEvent(QMouseEvent* e) override;

private:
	QWidget* m_window;
};

/// Hamburger-Knopf (nur im Platzsparmodus), Minimieren, Maximieren und Schließen.
class WindowControls : public QWidget
{
	Q_OBJECT
public:
	WindowControls(QWidget* window, QWidget* parent = nullptr);
	/// Symbol des Maximieren-Knopfs und Tooltip dem Fensterzustand anpassen.
	void updateState();
	/// Menü des Hamburger-Knopfs; ohne Menü bleibt der Knopf verborgen.
	void setBurgerMenu(QMenu* menu);
	void setBurgerVisible(bool on);
	QSize sizeHint() const override;

protected:
	void changeEvent(QEvent* e) override;

private:
	void paintBurgerIcon();
	QWidget* m_window;
	QToolButton* m_burger;
	QToolButton* m_max;
};

/// Fenster ein- oder ausmaximieren.
void toggleMaximized(QWidget* window);

/// Eigene Titelleiste: links `BrandStrip`, rechts `WindowControls`. Im Platzsparmodus wandern beide Teile
/// in die Tab-Leiste (siehe `MainWindow::setCompact`); `restoreParts` holt sie zurück.
/// Verschieben und Größe ändern übernimmt das Fenstersystem (`startSystemMove`, `startSystemResize`).
class TitleBar : public QWidget
{
	Q_OBJECT
public:
	TitleBar(QWidget* window, const QString& name, const QString& version, QWidget* parent = nullptr);
	void updateState();
	BrandStrip* brand() const { return m_brand; }
	WindowControls* controls() const { return m_controls; }
	/// Beide Teile wieder in die Titelleiste setzen.
	void restoreParts();

protected:
	void mousePressEvent(QMouseEvent* e) override;
	void mouseDoubleClickEvent(QMouseEvent* e) override;

private:
	QWidget* m_window;
	BrandStrip* m_brand;
	WindowControls* m_controls;
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
