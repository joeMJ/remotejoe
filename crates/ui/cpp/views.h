#pragma once
// Darstellung einer Sitzung: Bild (RDP/VNC) oder Terminal (SSH), mit Kopfzeile.

#include <QtCore/QElapsedTimer>
#include <QtCore/QFile>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonObject>
#include <QtCore/QSet>
#include <QtCore/QSize>
#include <QtCore/QTimer>
#include <QtGui/QColor>
#include <QtGui/QFont>
#include <QtGui/QImage>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QWidget>
#include <cstdint>
#include <functional>
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
	/// Mauszeiger der Gegenstelle (RDP/VNC): kind 0 = Standard, 1 = ausgeblendet, 2 = Bild (BGRA).
	void setRemotePointer(int kind, const QByteArray& bgra, int w, int h, int hx, int hy);
	/// Blauen Rahmen beim Tastaturfokus zeigen (Einstellung unter Extras → Darstellung).
	static void setFocusFrame(bool on) { s_focusFrame = on; }
	/// Web-Sitzung: Fokuswechsel an den Worker melden. Unter Windows läuft die Seite als Kindfenster dieses
	/// Widgets (WebView2 im Worker-Prozess): das Widget bekommt dafür ein eigenes natives Fenster.
	void setWeb(bool web);
	/// Windows: Elternfenster, Bereich, Sichtbarkeit und Kurzbefehle erneut an den Worker senden
	/// (nach dem Start oder Neustart des Workers; Nachrichten vor dem Start gehen verloren).
	void resyncWeb();
	/// Windows: Die Seite hat den Systemfokus bekommen (`on`) oder gibt ihn mit Tab ab (`forward`: weiter, sonst zurück).
	void webFocus(bool on, bool forward);
	/// Windows: Anwendungstaste aus der Seite (löst den Kurzbefehl des Hauptfensters aus).
	static void triggerShortcut(QWidget* window, int vk, bool ctrl, bool alt, bool shift);
	/// Windows: Das Widget ist ein eingebettetes Web-Fenster.
	bool isEmbedded() const { return m_embedded; }
	inline static bool s_focusFrame = true;
	/// Tastenkürzel des Desktops (Windows-Taste, Alt+Tab …) bei Fokus an RDP/VNC geben (Extras-Menü).
	inline static bool s_keyCapture = true;
	/// Hinweis in der Fußzeile, wenn die Tastenkürzel an die Sitzung gehen bzw. zurückgegeben wurden.
	inline static std::function<void(const QString&)> s_notice;
	/// Bildpunkt der Gegenstelle in Koordinaten dieses Widgets.
	QPoint toLocal(const QPoint& remote) const;

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
	void focusOutEvent(QFocusEvent*) override;
	bool focusNextPrevChild(bool) override { return false; }
	void showEvent(QShowEvent*) override;
	void hideEvent(QHideEvent*) override;
	bool event(QEvent*) override;

private:
	QPoint toRemote(const QPointF& p) const;
	/// Tastencode für den Worker: unter Linux Umschalt-, Windows- und Menütasten als Keysym (links/rechts).
	std::uint32_t keyCode(QKeyEvent* e) const;
	/// Tastenkürzel an diese Sitzung geben (nicht bei Web).
	void captureKeys();
	/// Alle noch gedrückten Tasten als losgelassen melden.
	void releasePressed();
	void webCommand(const QString& cmd);
	void sendBounds();
	bool m_web = false;
	bool m_embedded = false;
	QSet<std::uint32_t> m_pressed;
	/// Rechte Strg-Taste gedrückt und noch keine andere Taste dazu (Notausgang).
	bool m_rightCtrlAlone = false;
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
	/// Letzter Zeiger der Gegenstelle als Bild (unskaliert) und Klickpunkt; leer = Standardzeiger.
	QImage m_pointer;
	QPoint m_pointerHot;
	bool m_pointerHidden = false;
	void applyPointer();
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
	/// Schrift und Auswahlfarbe aus den Terminal-Einstellungen (JSON-Objekt des Backends).
	void applySettings(const QJsonObject& settings);
	/// Wendet die Einstellungen auf alle offenen Terminals an.
	static void applyToAll(const QJsonObject& settings);
	/// Das zuletzt benutzte, sichtbare Terminal (für das Menü Bearbeiten).
	static TerminalView* active();
	/// Markierten Text in die Zwischenablage kopieren (liefert, ob etwas markiert war).
	bool copySelection();
	/// Zwischenablage einfügen (bei mehreren Zeilen mit Rückfrage, falls eingestellt).
	void pasteClipboard();
	void selectAll();

protected:
	void paintEvent(QPaintEvent*) override;
	void keyPressEvent(QKeyEvent*) override;
	void mousePressEvent(QMouseEvent*) override;
	void mouseMoveEvent(QMouseEvent*) override;
	void mouseReleaseEvent(QMouseEvent*) override;
	void mouseDoubleClickEvent(QMouseEvent*) override;
	void contextMenuEvent(QContextMenuEvent*) override;
	void focusInEvent(QFocusEvent*) override;
	void resizeEvent(QResizeEvent*) override;
	/// Mausrad blättert im Rücklauf (bei less/top: Pfeiltasten); Strg + Mausrad ändert die Schriftgröße.
	void wheelEvent(QWheelEvent*) override;
	bool focusNextPrevChild(bool) override { return false; }

private:
	QPoint cellAt(const QPoint& p) const;
	QString selectedText();
	void pasteText(const QString& text);
	QStringList rowChars(int row) const;
	void selectWordAt(const QPoint& c);
	void selectLine(int row);
	void finishSelection();
	bool m_skipRelease = false;
	int m_healTries = 0;
	QElapsedTimer m_tripleClock;
	QPoint m_tripleCell{ -1, -1 };
	RjBackend* m_backend;
	std::uint64_t m_session;
	QJsonObject m_snap;
	QFont m_font;
	int m_cw = 8, m_ch = 16, m_ascent = 12;
	QPoint m_selStart{ -1, -1 }, m_selEnd{ -1, -1 };
	QSize m_lastSize;
	QColor m_selection;
	int m_wheel = 0;
	int m_scrollAcc = 0;
};

/// Rahmen um eine Sitzung mit Kopfzeile, Zustand und „Neu verbinden“.
class SessionPane : public QWidget
{
public:
	SessionPane(RjBackend* backend, std::uint64_t session, const QString& title, const QString& protocol,
	            QWidget* parent = nullptr);
	void setState(const QString& state, const QString& detail);
	void setNotice(const QString& text);
	QString notice() const { return m_noticeText; }
	/// Zustand für die Fußzeile des Hauptfensters; leer bei „Verbunden“.
	QString statusLine() const { return m_state.isEmpty() || m_state == QLatin1String("connected") ? QString() : m_stateText; }
	/// Schwerer Fehler (Verbindung fehlgeschlagen oder Worker abgestürzt).
	bool severe() const { return m_state == QLatin1String("failed") || m_state == QLatin1String("crashed"); }
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
	/// Öffnet den Editor eines Makros (setzt das Hauptfenster).
	void setMacroOpener(std::function<void(const QString&)> opener) { m_openMacro = std::move(opener); }
	/// Ergebnis eines Makrolaufs in der Kopfzeile anzeigen.
	void setMacroResult(const QString& text, bool ok);
	/// Web-Sitzung: Seitentitel, Laden/Fehler und Zurück/Vor aus dem Worker.
	void setWebStatus(const QJsonObject& e);
	/// Web-Sitzung: Menü des Workers (Kontextmenü, Auswahlliste) als Popup zeigen.
	void showWebMenu(const QJsonObject& e);
	/// Titel der Webseite (leer, solange keiner bekannt ist).
	QString pageTitle() const { return m_pageTitle; }
	bool isWeb() const { return m_back != nullptr; }
	/// Web-Sitzung: Suchfeld rechts in der Leiste öffnen (Strg+F).
	void openFind();
	/// Ergebnis der Suche (Feld rot ohne Treffer).
	void setFindResult(bool found);

private:
	void fillMacroMenu();
	void updateHeader();
	void webCommand(const QString& json);
	QString m_pageTitle;
	QString m_stateText;
	/// Ladezustand der Webseite für „lädt seit N s“ in der Kopfzeile.
	bool m_loading = false;
	int m_progress = 0;
	QElapsedTimer m_loadClock;
	QTimer m_loadTick;
	QPushButton* m_back = nullptr;
	QPushButton* m_forward = nullptr;
	QPushButton* m_reload = nullptr;
	/// Web: Adresse der Seite (markier- und kopierbar) und Suche in der Seite.
	class QLineEdit* m_url = nullptr;
	QWidget* m_findBox = nullptr;
	class QLineEdit* m_find = nullptr;
	void find(bool backwards, bool restart = false);
	RjBackend* m_backend;
	std::uint64_t m_session;
	QString m_title;
	QString m_state;
	std::function<void(const QString&)> m_openMacro;
	QLabel* m_macroState = nullptr;
	QPushButton* m_macroNew = nullptr;
	QPushButton* m_macroRun = nullptr;
	QPushButton* m_macroCancel = nullptr;
	QLabel* m_header;
	QLabel* m_notice;
	QString m_noticeText;
	QPushButton* m_reconnect;
	SecurityBar* m_security;
	ActionBar* m_ask;
	FrameView* m_frame = nullptr;
	TerminalView* m_terminal = nullptr;
};
