#pragma once
// Darstellung einer Sitzung: Bild (RDP/VNC) oder Terminal (SSH), mit Kopfzeile.

#include <QtCore/QFile>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonObject>
#include <QtCore/QSize>
#include <QtCore/QTimer>
#include <QtGui/QImage>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QWidget>
#include <cstdint>
#include <memory>

class RjBackend;
class SecurityBar;
class ActionBar;

/// Bild aus dem geteilten Speicher eines Workers.
class FrameView : public QWidget
{
public:
	FrameView(RjBackend* backend, std::uint64_t session, QWidget* parent = nullptr);
	~FrameView() override;
	/// Bildet den Bildpuffer ab (`fd`: Deskriptor bzw. Kennung, wird geschlossen).
	void attach(std::int64_t fd, int w, int h);
	/// Schließt eine nicht verwendete Kennung.
	static void closeHandle(std::int64_t fd);
	void damage(int x, int y, int w, int h);

protected:
	void paintEvent(QPaintEvent*) override;
	void resizeEvent(QResizeEvent*) override;
	void mouseMoveEvent(QMouseEvent*) override;
	void mousePressEvent(QMouseEvent*) override;
	void mouseReleaseEvent(QMouseEvent*) override;
	void wheelEvent(QWheelEvent*) override;
	void keyPressEvent(QKeyEvent*) override;
	void keyReleaseEvent(QKeyEvent*) override;
	void focusInEvent(QFocusEvent*) override;
	bool focusNextPrevChild(bool) override { return false; }

private:
	QPoint toRemote(const QPointF& p) const;
	QRect target() const;
	/// Meldet nach einer Pause die Größe des Fensters als Wunschauflösung der Sitzung
	/// (RDP: Anzeige-Steuerkanal oder Neuaufbau, VNC: ExtendedDesktopSize).
	void requestFit();
	RjBackend* m_backend;
	std::uint64_t m_session;
	void unmap();
	uchar* m_map = nullptr;
	std::size_t m_mapLen = 0;
	QImage m_image;
	QTimer* m_fitTimer = nullptr;
	QSize m_requested;
	QSize m_attached;
	int m_autoFits = 0;
};

/// Terminal (Zeichnung aus dem Gitter der Terminal-Zustandsmaschine).
class TerminalView : public QWidget
{
public:
	TerminalView(RjBackend* backend, std::uint64_t session, QWidget* parent = nullptr);
	void refresh();
	QSize cells() const;

protected:
	void paintEvent(QPaintEvent*) override;
	void keyPressEvent(QKeyEvent*) override;
	void mousePressEvent(QMouseEvent*) override;
	void mouseMoveEvent(QMouseEvent*) override;
	void mouseReleaseEvent(QMouseEvent*) override;
	void resizeEvent(QResizeEvent*) override;
	bool focusNextPrevChild(bool) override { return false; }

private:
	QPoint cellAt(const QPoint& p) const;
	RjBackend* m_backend;
	std::uint64_t m_session;
	QJsonObject m_snap;
	QFont m_font;
	int m_cw = 8, m_ch = 16, m_ascent = 12;
	QPoint m_selStart{ -1, -1 }, m_selEnd{ -1, -1 };
	QSize m_lastSize;
};

/// Rahmen um eine Sitzung mit Kopfzeile, Zustand und „Neu verbinden“.
class SessionPane : public QWidget
{
public:
	SessionPane(RjBackend* backend, std::uint64_t session, const QString& title, const QString& protocol,
	            QWidget* parent = nullptr);
	void setState(const QString& state, const QString& detail);
	void setNotice(const QString& text);
	FrameView* frame() const { return m_frame; }
	TerminalView* terminal() const { return m_terminal; }
	QWidget* view() const;
	/// Leiste für Host-Schlüssel- und Zertifikatsrückfragen dieser Sitzung.
	SecurityBar* security() const { return m_security; }
	/// Leiste für sonstige Rückfragen (z. B. Lesen der Zwischenablage).
	ActionBar* askBar() const { return m_ask; }
	std::uint64_t session() const { return m_session; }
	QString title() const { return m_title; }
	QString state() const { return m_state; }

private:
	RjBackend* m_backend;
	std::uint64_t m_session;
	QString m_title;
	QString m_state;
	QLabel* m_header;
	QLabel* m_notice;
	QPushButton* m_reconnect;
	SecurityBar* m_security;
	ActionBar* m_ask;
	FrameView* m_frame = nullptr;
	TerminalView* m_terminal = nullptr;
};
