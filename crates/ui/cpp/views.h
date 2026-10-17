#pragma once
// Darstellung einer Sitzung: Bild (RDP/VNC) oder Terminal (SSH), mit Kopfzeile.

#include <QtCore/QFile>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonObject>
#include <QtGui/QImage>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QWidget>
#include <cstdint>
#include <memory>

class RjBackend;

/// Bild aus dem geteilten Speicher eines Workers.
class FrameView : public QWidget
{
public:
	FrameView(RjBackend* backend, std::uint64_t session, QWidget* parent = nullptr);
	~FrameView() override;
	void attach(const QString& path, int w, int h);
	void damage(int x, int y, int w, int h);

protected:
	void paintEvent(QPaintEvent*) override;
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
	RjBackend* m_backend;
	std::uint64_t m_session;
	std::unique_ptr<QFile> m_file;
	uchar* m_map = nullptr;
	QImage m_image;
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
	FrameView* m_frame = nullptr;
	TerminalView* m_terminal = nullptr;
};
