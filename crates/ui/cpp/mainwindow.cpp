#include "mainwindow.h"
#include "faicons.h"
#include "mainwindow_p.h"

#include <QtWidgets/QTextBrowser>
#include <QtGui/QTextCursor>
#include <QtGui/QTextDocument>
#include <QtGui/QTextBlock>
#include <cmath>
#include <QtWidgets/QWidgetAction>
#include "rjpluginui.h"
#include "kbcapture.h"
#include "titlebar.h"
#include "exporttab.h"
#include "overviewtab.h"
#include "folderoverviewtab.h"
#include "orphantab.h"
#include "credmergetab.h"
#include "credchecktab.h"
#include "mcpserver.h"
#include "mcplogtab.h"
#include "appearance.h"
#include "settingstab.h"
#include "masstab.h"
#include "mergetab.h"
#include "sharedtab.h"
#include "reporttab.h"
#include <algorithm>

#include "editor.h"
#include "remotejoe-ui/src/bridge.cxxqt.h"
#include "views.h"
#include <cstdio>

#include <QtCore/QDateTime>
#include <QtCore/QDir>
#include <QtCore/QSettings>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QElapsedTimer>
#include <QtGui/QAction>
#include <QtGui/QActionGroup>
#include <QtCore/QCryptographicHash>
#include <QtCore/QMimeData>
#include <QtGui/QClipboard>
#include <QtWidgets/QInputDialog>
#include <QtGui/QDropEvent>
#include <QtGui/QCloseEvent>
#include <QtGui/QFontDatabase>
#include <QtGui/QGuiApplication>
#include <QtGui/QKeySequence>
#include <QtGui/QPainter>
#include <QtGui/QPainterPath>
#include <QtGui/QShortcut>
#include <QtSvg/QSvgRenderer>
#include <QtWidgets/QApplication>
#include <QtWidgets/QAbstractButton>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QToolButton>
#include <QtGui/QScreen>
#include <QtGui/QResizeEvent>
#include <QtWidgets/QComboBox>
#include <QtCore/QPointer>
#include <QtWidgets/QFileDialog>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QDialog>
#include <QtWidgets/QDialogButtonBox>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QMenu>
#include <QtWidgets/QScrollBar>
#include <functional>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QMenuBar>
#include <QtWidgets/QSplitter>
#include <QtWidgets/QStatusBar>
#include <QtWidgets/QStyleFactory>
#include <QtGui/QStyleHints>
#include <QtGui/QDesktopServices>
#include <QtGui/QWindow>
#include <QtWidgets/QTabBar>
#include <QtGui/QHelpEvent>
#include <QtWidgets/QToolTip>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QProgressBar>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QVBoxLayout>
#include <QtCore/QAbstractNativeEventFilter>
#ifdef Q_OS_LINUX
#include <QtDBus/QDBusConnection>
#endif
#ifdef Q_OS_WIN
#include <windows.h>
#include <windowsx.h>
#endif

QElapsedTimer MainWindow::startClock;

using namespace rjmw;

namespace {


QString T(RjBackend* b, const char* key)
{
	return b->text(QString::fromLatin1(key));
}

QJsonObject parseObject(const QString& s)
{
	return QJsonDocument::fromJson(s.toUtf8()).object();
}

/// Symbol je Protokoll (SVG mit Kürzel), optional mit Ampelpunkt.
/// Zeichnet ein kleines SVG-Symbol (Körper mit dem Platzhalter COLOR) in der Textfarbe.
QPixmap svgPixmap(const char* body, const QColor& color, int px)
{
	const QString svg = QStringLiteral("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'>") +
	                    QString::fromLatin1(body).replace(QLatin1String("COLOR"), color.name()) + QStringLiteral("</svg>");
	QSvgRenderer renderer(svg.toUtf8());
	const qreal dpr = qApp->devicePixelRatio();
	QPixmap pm(QSize(px, px) * dpr);
	pm.setDevicePixelRatio(dpr);
	pm.fill(Qt::transparent);
	QPainter p(&pm);
	renderer.render(&p, QRectF(0, 0, px, px));
	return pm;
}

const char* const kSvgTarget = "<rect x='1.5' y='2.5' width='13' height='8.5' rx='1' fill='none' stroke='COLOR' stroke-width='1.3'/>"
                               "<path d='M5 14h6M8 11v3' stroke='COLOR' stroke-width='1.3' fill='none' stroke-linecap='round'/>";
const char* const kSvgClock = "<circle cx='8' cy='8' r='6' fill='none' stroke='COLOR' stroke-width='1.3'/>"
                              "<path d='M8 4.5V8l2.5 1.5' stroke='COLOR' stroke-width='1.3' fill='none' stroke-linecap='round' stroke-linejoin='round'/>";
const char* const kSvgUser = "<circle cx='8' cy='5.5' r='2.7' fill='none' stroke='COLOR' stroke-width='1.3'/>"
                             "<path d='M2.8 14c0-3 2.3-4.8 5.2-4.8s5.2 1.8 5.2 4.8' stroke='COLOR' stroke-width='1.3' fill='none' stroke-linecap='round'/>";

QString protocolName(const QString& p)
{
	static const QHash<QString, QString> names = { { QStringLiteral("ssh"), QStringLiteral("SSH") },     { QStringLiteral("rdp"), QStringLiteral("RDP") },
		                                           { QStringLiteral("vnc"), QStringLiteral("VNC") },     { QStringLiteral("web"), QStringLiteral("Web") },
		                                           { QStringLiteral("psrp"), QStringLiteral("PowerShell") }, { QStringLiteral("proxmox"), QStringLiteral("Proxmox") },
		                                           { QStringLiteral("hyperv"), QStringLiteral("Hyper-V") } };
	return names.value(p, p.toUpper());
}

// Symbolfarben des aktuellen Themes (Art → Farbe) und Ringfarbe des Statuspunkts; eine Änderung
// verwirft die schon gezeichneten Symbole.
QHash<QString, QColor> g_iconColors;
QColor g_iconRing(Qt::white);
// Ausgewählte Zeile: Symbol in der Textfarbe der Auswahl, Ring in der Auswahlfarbe.
QColor g_iconSelected(Qt::white);
QColor g_iconSelectedRing(QColor("#2a6fdb"));
int g_iconGeneration = 0;

void setIconColors(const QJsonObject& colors, const QColor& ring, const QColor& selected, const QColor& selectedRing)
{
	g_iconSelected = selected;
	g_iconSelectedRing = selectedRing;
	g_iconColors.clear();
	for (auto it = colors.begin(); it != colors.end(); ++it)
		g_iconColors.insert(it.key(), QColor(it.value().toString()));
	g_iconRing = ring;
	++g_iconGeneration;
}

/// `session`: 0 = keine offene Sitzung, 1 = verbunden (gefüllter Punkt oben rechts),
/// 2 = Tab offen, aber nicht verbunden (Ring).
QIcon protocolIcon(const QString& protocol, const QString& light, bool folder, int session = 0)
{
	static QHash<QString, QIcon> cache;
	static int generation = -1;
	if (generation != g_iconGeneration)
	{
		cache.clear();
		generation = g_iconGeneration;
	}
	const QString key = protocol + QLatin1Char('|') + light + (folder ? QStringLiteral("|f") : QString()) + QLatin1Char('|') + QString::number(session);
	auto it = cache.find(key);
	if (it != cache.end())
		return *it;
	struct P
	{
		const char* name;
		const char* fa; // Font-Awesome-Symbol (siehe faicons.h)
		const char* color;
	};
	// Sprechende Symbole aus Font Awesome; die Farbe je Zieltyp kommt weiter aus dem Farbschema.
	static const P table[] = {
		{ "rdp", "display", "#0b5cad" },
		{ "ssh", "terminal", "#1b6e20" },
		{ "vnc", "desktop", "#7a3e9d" },
		{ "web", "globe", "#8a5300" },
		{ "psrp", "code", "#005f73" },
		{ "proxmox", "cubes", "#b03a00" },
		{ "hyperv", "layer-group", "#3a4a8c" },
		{ "login", "key", "#6a1b9a" },
		{ "controller", "wifi", "#00897b" },
		{ "switch", "network-wired", "#00695c" },
		{ "site", "building", "#6d4c41" },
		{ "tplfolder", "r-clone", "#5c6bc0" },
		{ "netfolder", "sitemap", "#00695c" },
		{ "srvfolder", "server", "#37474f" },
		{ "multi", "r-window-restore", "#0b5cad" },
		{ "macro", "play", "#6a1b9a" },
	};
	// Symbolname und Farbe; `over` ersetzt die Farbe (ausgewählte Zeile), `open` = aufgeklappter Ordner.
	auto pick = [&](const QString& over, bool open) -> QPair<QString, QColor> {
		if (folder)
			return { open ? QStringLiteral("r-folder-open") : QStringLiteral("r-folder"),
			         over.isEmpty() ? g_iconColors.value(QStringLiteral("folder"), QColor("#c9a227")) : QColor(over) };
		QString fa = QStringLiteral("circle-question");
		QColor color = g_iconColors.value(QStringLiteral("other"), QColor("#555555"));
		for (const P& p : table)
			if (protocol == QLatin1String(p.name))
			{
				fa = QLatin1String(p.fa);
				color = g_iconColors.value(protocol, QColor(QLatin1String(p.color)));
			}
		return { fa, over.isEmpty() ? color : QColor(over) };
	};
	auto draw = [&](const QPair<QString, QColor>& sym, const QColor& ring) {
	QPixmap pm(32, 32);
	pm.fill(Qt::transparent);
	{
		QPainter p(&pm);
		faPaint(&p, QRectF(2, 3, 26, 26), sym.first, sym.second);
		if (!light.isEmpty())
		{
			QColor c = light == QLatin1String("green")    ? QColor("#1b9e20")
			           : light == QLatin1String("yellow") ? QColor("#e0a800")
			           : light == QLatin1String("red")    ? QColor("#d32f2f")
			                                              : QColor("#8a8a8a");
			p.setRenderHint(QPainter::Antialiasing);
			p.setBrush(c);
			p.setPen(QPen(ring, 2));
			p.drawEllipse(QRect(19, 19, 12, 12));
		}
		if (session > 0)
		{
			const QColor dot = g_iconColors.value(QStringLiteral("session"), QColor("#2e9e44"));
			p.setRenderHint(QPainter::Antialiasing);
			if (session == 1)
			{
				p.setBrush(dot);
				p.setPen(QPen(ring, 2));
				p.drawEllipse(QRect(20, 0, 12, 12));
			}
			else
			{
				p.setBrush(ring);
				p.setPen(QPen(dot, 2.5));
				p.drawEllipse(QRectF(21.25, 1.25, 9.5, 9.5));
			}
		}
	}
	return pm;
	};
	QIcon icon(draw(pick(QString(), false), g_iconRing));
	icon.addPixmap(draw(pick(g_iconSelected.name(), false), g_iconSelectedRing), QIcon::Selected);
	if (folder)
	{
		// Aufgeklappte Ordner zeigen den offenen Ordner (der Baum fragt den Zustand „On“ ab).
		icon.addPixmap(draw(pick(QString(), true), g_iconRing), QIcon::Normal, QIcon::On);
		icon.addPixmap(draw(pick(g_iconSelected.name(), true), g_iconSelectedRing), QIcon::Selected, QIcon::On);
	}
	cache.insert(key, icon);
	return icon;
}

} // namespace

/// Symbol eines virtuellen Switches: Switch in Gelb (Baum, Netzwerkgeräte).
QIcon rjVirtualSwitchIcon()
{
	QPixmap px(36, 36);
	px.fill(Qt::transparent);
	QPainter p(&px);
	p.setRenderHint(QPainter::Antialiasing);
	p.setPen(QPen(QColor(133, 79, 11), 2));
	p.setBrush(QColor(239, 159, 39));
	p.drawRoundedRect(QRectF(3, 11, 30, 14), 3, 3);
	for (int i = 0; i < 6; ++i)
		p.fillRect(QRectF(7 + i * 4, 16, 2.5, 4), QColor(99, 56, 6));
	return QIcon(px);
}

namespace
{
void addNodes(QStandardItem* parent, const QJsonArray& nodes)
{
	for (const QJsonValue& v : nodes)
	{
		const QJsonObject n = v.toObject();
		const bool folder = n.value(QStringLiteral("folder")).toBool();
		auto* item = new QStandardItem(n.value(QStringLiteral("name")).toString());
		item->setEditable(false);
		item->setData(n.value(QStringLiteral("id")).toString(), RoleId);
		item->setData(folder, RoleFolder);
		// Symbol: erkannte Geräteart vor „mehrere Zugangswege“ vor dem Protokoll.
		const bool site = n.value(QStringLiteral("site")).toBool();
		const QString device = n.value(QStringLiteral("device")).toString();
		// Ordner mit Rolle (Vorlagen, Netzwerk-, Serverkomponenten): eigenes Piktogramm statt des Ordners.
		const QString role = n.value(QStringLiteral("role")).toString();
		const QString roleKind = role == QLatin1String("network")   ? QStringLiteral("netfolder")
		                         : role == QLatin1String("servers") ? QStringLiteral("srvfolder")
		                         : !role.isEmpty()                  ? QStringLiteral("tplfolder")
		                                                            : QString();
		const QString iconKind = site                                                                         ? QStringLiteral("site")
		                         : !roleKind.isEmpty()                                                       ? roleKind
		                         : device == QLatin1String("switch")                                           ? QStringLiteral("switch")
		                         : device == QLatin1String("controller")                                       ? QStringLiteral("controller")
		                         : n.value(QStringLiteral("multi")).toBool()                                ? QStringLiteral("multi")
		                                                                                                    : n.value(QStringLiteral("protocol")).toString();
		item->setData(iconKind, RoleIconKind);
		item->setData(site, RoleSite);
		item->setIcon(protocolIcon(iconKind, n.value(QStringLiteral("light")).toString(), folder && !site && roleKind.isEmpty()));
		item->setData(n.value(QStringLiteral("light")).toString(), RoleLight);
		if (n.value(QStringLiteral("vswitch")).toBool())
		{
			item->setData(n.value(QStringLiteral("name")).toString(), RoleVSwitch);
			item->setText(n.value(QStringLiteral("name")).toString() + QStringLiteral(" (virtuell)"));
			item->setIcon(rjVirtualSwitchIcon());
			item->setForeground(QColor(133, 79, 11));
			item->setDragEnabled(false);
			item->setDropEnabled(false);
		}
		if (n.value(QStringLiteral("login")).toBool())
		{
			item->setData(true, RoleLogin);
			item->setIcon(protocolIcon(QStringLiteral("login"), QString(), false));
			// Ziehen erlaubt (Reihenfolge, anderer Ordner); auf ein Anmeldeobjekt ablegen nicht.
			item->setDropEnabled(false);
		}
		QString tip = n.value(QStringLiteral("host")).toString();
		if (n.value(QStringLiteral("suspicious")).toBool())
			tip += QStringLiteral("\nName enthält unsichtbare oder richtungsändernde Zeichen.");
		item->setToolTip(tip);
		const QString proto = n.value(QStringLiteral("protocol")).toString();
		item->setData(proto, RoleProtocol);
		item->setData(n.value(QStringLiteral("other_way")).toBool(), RoleOtherWay);
		parent->appendRow(item);
		addNodes(item, n.value(QStringLiteral("children")).toArray());
		if (proto == QStringLiteral("proxmox") || proto == QStringLiteral("hyperv"))
		{
			auto* ph = new QStandardItem(QStringLiteral("…"));
			ph->setEditable(false);
			ph->setData(true, RolePlaceholder);
			ph->setData(true, RoleFolder);
			item->appendRow(ph);
		}
	}
}

#ifdef Q_OS_WIN
/// Energiestatus-Meldungen des Fensters (Aufwachen aus dem Standby).
class PowerFilter : public QAbstractNativeEventFilter
{
public:
	explicit PowerFilter(std::function<void()> resumed)
	  : m_resumed(std::move(resumed))
	{
	}
	bool nativeEventFilter(const QByteArray& type, void* message, qintptr*) override
	{
		if (type == "windows_generic_MSG")
		{
			const MSG* m = static_cast<const MSG*>(message);
			if (m->message == WM_POWERBROADCAST && (m->wParam == PBT_APMRESUMEAUTOMATIC || m->wParam == PBT_APMRESUMESUSPEND))
				m_resumed();
		}
		return false;
	}

private:
	std::function<void()> m_resumed;
};

/// Nativer Größenrahmen ohne Titelleiste: Das Fenster bleibt für Qt rahmenlos (eigene Titelleiste), bekommt aber
/// WS_THICKFRAME, WS_CAPTION, Minimieren und Maximieren (Aero Snap, Win+Pfeile). WM_NCCALCSIZE macht das ganze
/// Fenster zum Inhaltsbereich; WM_NCHITTEST meldet Ränder und Ecken in der Breite des Systemrahmens (je Bildschirm-
/// DPI), dann stellt Windows Zeiger und Größenänderung selbst. Der Innenrand (gleiche Breite) hält eigene native
/// Fenster (WebView2) aus dieser Zone; maximiert gleicht er den Überstand über den Bildschirm aus.
class WinFrame : public QObject, public QAbstractNativeEventFilter
{
public:
	WinFrame(QWidget* window, std::function<void(int)> setInset)
	  : QObject(window)
	  , m_window(window)
	  , m_setInset(std::move(setInset))
	{
		qApp->installNativeEventFilter(this);
		window->installEventFilter(this);
		attach();
	}
	~WinFrame() override { qApp->removeNativeEventFilter(this); }

	bool eventFilter(QObject* o, QEvent* e) override
	{
		// Qt kann das native Fenster neu anlegen oder den Stil zurücksetzen (Vollbild): Stil und Innenrand erneuern.
		if (o == m_window && (e->type() == QEvent::WinIdChange || e->type() == QEvent::Show || e->type() == QEvent::WindowStateChange))
			QTimer::singleShot(0, this, [this] { attach(); });
		return false;
	}

	bool nativeEventFilter(const QByteArray& type, void* message, qintptr* result) override
	{
		if (type != "windows_generic_MSG")
			return false;
		const MSG* m = static_cast<const MSG*>(message);
		if (!m_hwnd || m->hwnd != m_hwnd)
			return false;
		switch (m->message)
		{
			case WM_NCCALCSIZE:
				if (m->wParam && !m_window->isFullScreen())
				{
					*result = 0;
					return true;
				}
				break;
			case WM_NCHITTEST:
			{
				if (IsZoomed(m_hwnd) || m_window->isFullScreen())
					break;
				RECT r;
				GetWindowRect(m_hwnd, &r);
				const int x = GET_X_LPARAM(m->lParam), y = GET_Y_LPARAM(m->lParam);
				const int b = border(), c = 2 * b;
				const bool left = x < r.left + b, right = x >= r.right - b, top = y < r.top + b, bottom = y >= r.bottom - b;
				// Ecken etwas größer, damit sie leicht zu treffen sind.
				const bool nearLeft = x < r.left + c, nearRight = x >= r.right - c, nearTop = y < r.top + c, nearBottom = y >= r.bottom - c;
				LRESULT hit = 0;
				if ((top && nearLeft) || (left && nearTop))
					hit = HTTOPLEFT;
				else if ((top && nearRight) || (right && nearTop))
					hit = HTTOPRIGHT;
				else if ((bottom && nearLeft) || (left && nearBottom))
					hit = HTBOTTOMLEFT;
				else if ((bottom && nearRight) || (right && nearBottom))
					hit = HTBOTTOMRIGHT;
				else if (left)
					hit = HTLEFT;
				else if (right)
					hit = HTRIGHT;
				else if (top)
					hit = HTTOP;
				else if (bottom)
					hit = HTBOTTOM;
				if (hit)
				{
					*result = hit;
					return true;
				}
				break;
			}
			case WM_SIZE:
			case WM_DPICHANGED:
				QTimer::singleShot(0, this, [this] { updateInset(); });
				break;
			default:
				break;
		}
		return false;
	}

private:
	/// Breite der Größenzone in Gerätepixeln (Systemrahmen für die DPI des Bildschirms, bei 100 % 8 px).
	int border() const
	{
		const UINT dpi = GetDpiForWindow(m_hwnd);
		return GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
	}

	void attach()
	{
		if (!m_window->windowHandle())
			m_window->winId();
		m_hwnd = reinterpret_cast<HWND>(m_window->winId());
		if (!m_hwnd || m_window->isFullScreen())
		{
			updateInset();
			return;
		}
		applyCornerPreference();
		const LONG_PTR style = GetWindowLongPtrW(m_hwnd, GWL_STYLE);
		const LONG_PTR want = style | WS_THICKFRAME | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
		if (want != style)
		{
			SetWindowLongPtrW(m_hwnd, GWL_STYLE, want);
			SetWindowPos(m_hwnd, nullptr, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE);
		}
		updateInset();
	}

	/// Windows 11: runde oder eckige Fensterecken nach der Einstellung „Runde Fensterecken“ (DWMWA_WINDOW_CORNER_PREFERENCE).
	/// dwmapi wird zur Laufzeit geladen; ältere Windows-Fassungen kennen das Attribut nicht und ignorieren es.
	void applyCornerPreference()
	{
		using SetAttr = HRESULT(WINAPI*)(HWND, DWORD, LPCVOID, DWORD);
		static SetAttr set = [] {
			HMODULE dwm = LoadLibraryW(L"dwmapi.dll");
			return dwm ? reinterpret_cast<SetAttr>(GetProcAddress(dwm, "DwmSetWindowAttribute")) : nullptr;
		}();
		if (!set)
			return;
		const bool round = m_window->property("rjRoundCorners").toBool();
		const DWORD pref = round ? 2 /* DWMWCP_ROUND */ : 1 /* DWMWCP_DONOTROUND */;
		set(m_hwnd, 33 /* DWMWA_WINDOW_CORNER_PREFERENCE */, &pref, sizeof(pref));
	}

	/// Innenrand in logischen Bildpunkten: normal die Größenzone, maximiert der Überstand über den Arbeitsbereich.
	void updateInset()
	{
		if (!m_hwnd)
			return;
		int px = 0;
		if (m_window->isFullScreen())
			px = 0;
		else if (IsZoomed(m_hwnd))
		{
			RECT r;
			GetWindowRect(m_hwnd, &r);
			MONITORINFO mi{};
			mi.cbSize = sizeof(mi);
			if (GetMonitorInfoW(MonitorFromWindow(m_hwnd, MONITOR_DEFAULTTONEAREST), &mi))
				px = qMax(0, int(mi.rcWork.left - r.left));
		}
		// Normalzustand: kein Innenrand – Tabs und Baum reichen bis an den Fensterrand (wie Firefox); die Größenzone
		// meldet WM_NCHITTEST trotzdem. Maximiert gleicht der Innenrand den Überstand über den Arbeitsbereich aus.
		const qreal dpr = m_window->devicePixelRatioF();
		const int inset = qRound(px / (dpr > 0 ? dpr : 1.0));
		if (inset != m_inset)
		{
			m_inset = inset;
			m_setInset(inset);
		}
		updateRegion();
	}

	/// Abgerundete Fensterform, wo Windows selbst nicht rundet (Windows 10, Remotedesktop-Sitzung): Ecken mit 8 px
	/// (je DPI). Unter Windows 11 am eigenen Bildschirm rundet DWM sauber; maximiert und im Vollbild eckig.
	void updateRegion()
	{
		if (!m_hwnd)
			return;
		static const bool needRegion = [] {
			OSVERSIONINFOEXW v{};
			v.dwOSVersionInfoSize = sizeof(v);
			using RtlGetVersion = LONG(WINAPI*)(OSVERSIONINFOEXW*);
			bool win11 = false;
			if (HMODULE nt = GetModuleHandleW(L"ntdll.dll"))
				if (auto f = reinterpret_cast<RtlGetVersion>(GetProcAddress(nt, "RtlGetVersion")); f && f(&v) == 0)
					win11 = v.dwMajorVersion >= 10 && v.dwBuildNumber >= 22000;
			return !win11 || GetSystemMetrics(SM_REMOTESESSION) != 0;
		}();
		const bool round = m_window->property("rjRoundCorners").toBool() && needRegion && !IsZoomed(m_hwnd) && !m_window->isFullScreen();
		if (!round)
		{
			if (m_region)
			{
				SetWindowRgn(m_hwnd, nullptr, TRUE);
				m_region = false;
			}
			return;
		}
		RECT r;
		GetWindowRect(m_hwnd, &r);
		const int d = MulDiv(16, GetDpiForWindow(m_hwnd), 96);
		HRGN rgn = CreateRoundRectRgn(0, 0, r.right - r.left + 1, r.bottom - r.top + 1, d, d);
		SetWindowRgn(m_hwnd, rgn, TRUE); // Windows übernimmt die Region
		m_region = true;
	}

	QWidget* m_window;
	std::function<void(int)> m_setInset;
	bool m_region = false;
	HWND m_hwnd = nullptr;
	int m_inset = -1;
};
#endif

/// Baum mit Ziehen und Ablegen: verschoben wird im Index, nicht im Modell.
class RjTree : public QTreeView
{
public:
	using QTreeView::QTreeView;
	/// Lage: 0 auf, 1 über, 2 unter dem Eintrag, 3 freie Fläche.
	std::function<void(const QModelIndex& source, const QModelIndex& target, int position)> onDrop;

protected:
	void dropEvent(QDropEvent* e) override
	{
		const QModelIndex src = currentIndex();
		const QModelIndex dst = indexAt(e->position().toPoint());
		const int pos = int(dropIndicatorPosition());
		e->setDropAction(Qt::IgnoreAction);
		e->accept();
		if (onDrop && src.isValid())
			onDrop(src, dst, pos);
	}
};

/// Ziel-IDs der Auswahl (Ordner schließen ihre Ziele ein).
QStringList selectedTargets(QTreeView* tree, QStandardItemModel* model)
{
	QStringList ids;
	std::function<void(const QModelIndex&)> collect = [&](const QModelIndex& i) {
		if (!i.data(RoleFolder).toBool())
		{
			const QString id = i.data(RoleId).toString();
			if (!ids.contains(id))
				ids << id;
			return;
		}
		for (int r = 0; r < model->rowCount(i); ++r)
			collect(model->index(r, 0, i));
	};
	for (const QModelIndex& i : tree->selectionModel()->selectedIndexes())
		collect(i);
	return ids;
}

} // namespace

/// Symbol wie im Baum (Geräteart, Ampel) – auch für Tabellen der Module.
QIcon rjDeviceIcon(const QString& kind, const QString& light)
{
	return protocolIcon(kind, light, false);
}

void MainWindow::openPatchday(const QStringList& ids)
{
	const QString res = m_backend->startPatchday(QString::fromUtf8(QJsonDocument(QJsonArray::fromStringList(ids)).toJson(QJsonDocument::Compact)), false);
	statusBar()->showMessage(res, 5000);
	auto* t = new PatchdayTab(m_backend, ids, m_tabs);
	addToolTab(t, T(m_backend, "patch.title"));
}

QString MainWindow::patchdayMeasurement() const
{
	auto* p = findChild<PatchdayTab*>();
	return p ? QStringLiteral("patchday_neuzeichnen_max_ms=%1\npatchday_neuzeichnen_anzahl=%2\n").arg(p->maxRefreshMs).arg(p->refreshes) : QString();
}

void MainWindow::patchdayForFirstTargets(int n)
{
	QStringList ids;
	std::function<void(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent) && ids.size() < n; ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			if (i.data(RoleFolder).toBool())
				walk(i);
			else
				ids << i.data(RoleId).toString();
		}
	};
	walk(QModelIndex());
	openPatchday(ids);
}

void MainWindow::openMerge(const QStringList& ids, const QString& title, bool manual)
{
	// Eine Vorschau zur Zeit: eine vorhandene wird ersetzt.
	if (auto* old = findChild<MergeTab*>())
		closeToolTab(old);
	auto* t = new MergeTab(m_backend, ids, title, manual, m_tabs);
	connect(t, &MergeTab::showTarget, this, [this](const QString& id) { selectId(id); });
	addToolTab(t, T(m_backend, "merge.tab"));
}

void MainWindow::openSharedCheck()
{
	if (auto* old = findChild<SharedCredTab*>())
	{
		old->load();
		m_tabs->setCurrentWidget(old);
		return;
	}
	auto* t = new SharedCredTab(m_backend, m_tabs);
	connect(t, &SharedCredTab::showTarget, this, [this](const QString& id) { selectId(id); });
	addToolTab(t, T(m_backend, "shared.tab"));
}

void MainWindow::mergeForFirstFolder()
{
	const QModelIndex i = m_model->index(0, 0, QModelIndex());
	if (i.isValid())
		openMerge({ i.data(RoleId).toString() }, i.data().toString());
}

namespace {
/// Breite des Randes, an dem das Fenster in der Größe verändert wird (Pixel).
constexpr int kResizeBorder = 8;

/// Kleiner Schiebeschalter für die Fußzeile.
class SwitchButton : public QAbstractButton
{
public:
	explicit SwitchButton(QWidget* parent)
	  : QAbstractButton(parent)
	{
		setCheckable(true);
		setFixedSize(26, 12);
		setCursor(Qt::PointingHandCursor);
		setFocusPolicy(Qt::NoFocus);
	}

protected:
	void paintEvent(QPaintEvent*) override
	{
		QPainter p(this);
		p.setRenderHint(QPainter::Antialiasing);
		const QRectF r = QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5);
		p.setPen(Qt::NoPen);
		p.setBrush(isChecked() ? palette().highlight() : palette().mid());
		p.drawRoundedRect(r, r.height() / 2, r.height() / 2);
		const qreal d = r.height() - 4;
		p.setBrush(palette().base());
		p.drawEllipse(QRectF(isChecked() ? r.right() - d - 2 : r.left() + 2, r.top() + 2, d, d));
	}
};

#ifdef Q_OS_WIN
/// WebView2-Laufzeit vorhanden? Nach Microsoft: Wert „pv“ des Laufzeit-Clients (systemweit oder je Benutzer),
/// weder leer noch 0.0.0.0.
bool webView2Installed()
{
	const QString client = QStringLiteral("Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}");
	for (const QString& root : { QStringLiteral("HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\"), QStringLiteral("HKEY_LOCAL_MACHINE\\SOFTWARE\\"),
	                             QStringLiteral("HKEY_CURRENT_USER\\Software\\") })
	{
		const QString pv = QSettings(root + client, QSettings::NativeFormat).value(QStringLiteral("pv")).toString();
		if (!pv.isEmpty() && pv != QLatin1String("0.0.0.0"))
			return true;
	}
	return false;
}

/// Ergebnis eines Updates über die MSI (Neustart): erscheint nach dem Entsperren als Hinweis; vorher verdeckt
/// die Entsperr-Abfrage sie.
QString g_updateNotice;
#endif
} // namespace

namespace
{
/// Halbdurchsichtige Fläche über dem Fenster mit Pfeilen auf Menü und „Hinzufügen“ (Ersthilfe).
class CoachOverlay : public QWidget
{
public:
	struct Hint
	{
		QPointer<QWidget> target;
		QString text;
	};
	CoachOverlay(QWidget* parent, QList<Hint> hints, const QString& dontShow, const QString& ok, std::function<void(bool)> done)
	  : QWidget(parent)
	  , m_hints(std::move(hints))
	  , m_done(std::move(done))
	{
		setAttribute(Qt::WA_StyledBackground, false);
		m_check = new QCheckBox(dontShow, this);
		m_ok = new QPushButton(ok, this);
		m_ok->setDefault(true);
		m_check->setStyleSheet(QStringLiteral("QCheckBox { color: white; }"));
		connect(m_ok, &QPushButton::clicked, this, [this] {
			if (m_done)
				m_done(m_check->isChecked());
			deleteLater();
		});
		parent->installEventFilter(this);
		setGeometry(parent->rect());
		show();
		raise();
		m_ok->setFocus();
	}
	bool eventFilter(QObject* o, QEvent* e) override
	{
		if (o == parentWidget() && e->type() == QEvent::Resize)
			setGeometry(parentWidget()->rect());
		return QWidget::eventFilter(o, e);
	}

protected:
	void resizeEvent(QResizeEvent*) override
	{
		const QSize c = m_check->sizeHint(), b = m_ok->sizeHint();
		const int y = height() - 70;
		m_check->setGeometry((width() - c.width() - b.width() - 24) / 2, y + (b.height() - c.height()) / 2, c.width(), c.height());
		m_ok->setGeometry(m_check->geometry().right() + 24, y, b.width() + 20, b.height());
	}
	void paintEvent(QPaintEvent*) override
	{
		QPainter p(this);
		p.setRenderHint(QPainter::Antialiasing);
		p.fillRect(rect(), QColor(0, 0, 0, 150));
		const QColor accent = palette().color(QPalette::Highlight);
		const QColor boxBg = palette().color(QPalette::Base), boxText = palette().color(QPalette::Text);
		int slot = 0;
		m_nextY = 0;
		for (const Hint& h : m_hints)
		{
			if (!h.target || !h.target->isVisible())
				continue;
			// Über Bildschirmkoordinaten: die Hinweisfläche ist Geschwister, nicht Elternteil des Ziels.
			const QRect tr(mapFromGlobal(h.target->mapToGlobal(QPoint(0, 0))), h.target->size());
			// Ziel hervorheben.
			p.setPen(QPen(accent, 3));
			p.setBrush(Qt::NoBrush);
			p.drawRoundedRect(QRectF(tr).adjusted(-5, -5, 5, 5), 8, 8);
			// Textkasten: links unterhalb des Ziels bzw. darunter, innerhalb des Fensters.
			QFont f = font();
			f.setPointSizeF(f.pointSizeF() * 1.1);
			p.setFont(f);
			const QFontMetrics fm(f);
			const int w = 340;
			const QRect textRect = fm.boundingRect(QRect(0, 0, w - 32, 1000), Qt::TextWordWrap, h.text);
			const int bw = w, bh = textRect.height() + 28;
			// Großes Ziel (Baum): Kasten rechts daneben auf halber Höhe, Pfeil in die Mitte.
			const bool big = tr.height() > 120;
			int bx = tr.center().x() - bw - 60 - slot * 40, by = qMax(tr.bottom() + 70, m_nextY);
			if (big)
			{
				bx = qMin(width() - bw - 20, tr.right() + 90);
				by = qMax(m_nextY, tr.center().y() - bh / 2);
			}
			else if (bx < 20)
				bx = qMin(width() - bw - 20, tr.right() + 60);
			by = qMin(by, height() - bh - 110);
			m_nextY = by + bh + 40;
			const QRectF box(bx, by, bw, bh);
			p.setPen(Qt::NoPen);
			p.setBrush(boxBg);
			p.drawRoundedRect(box, 10, 10);
			p.setPen(boxText);
			p.drawText(box.adjusted(16, 14, -16, -14), Qt::TextWordWrap, h.text);
			// Pfeil vom Kasten zum Ziel.
			const QPointF from = big ? QPointF(box.left(), box.center().y()) : QPointF(box.center().x() > tr.center().x() ? box.left() + 30 : box.right() - 30, box.top());
			const QPointF to = big ? QPointF(tr.center().x() + 20, tr.center().y()) : QPointF(tr.center().x(), tr.bottom() + 8);
			QPainterPath path(from);
			if (big)
				path.lineTo(to);
			else
				path.cubicTo(QPointF(from.x(), (from.y() + to.y()) / 2), QPointF(to.x(), (from.y() + to.y()) / 2 + 20), to);
			p.setPen(QPen(accent, 3, Qt::SolidLine, Qt::RoundCap));
			p.setBrush(Qt::NoBrush);
			p.drawPath(path);
			const qreal a = big ? std::atan2(to.y() - from.y(), to.x() - from.x()) : std::atan2(to.y() - (from.y() + to.y()) / 2 - 20, 0.0001);
			QPolygonF head;
			head << to << to + QPointF(std::cos(a + 2.6) * 14, std::sin(a + 2.6) * 14) << to + QPointF(std::cos(a - 2.6) * 14, std::sin(a - 2.6) * 14);
			p.setBrush(accent);
			p.drawPolygon(head);
			++slot;
		}
	}
	void mousePressEvent(QMouseEvent* e) override { e->accept(); }

private:
	QList<Hint> m_hints;
	std::function<void(bool)> m_done;
	QCheckBox* m_check;
	QPushButton* m_ok;
	int m_nextY = 0;
};
} // namespace

namespace
{
/// Tableiste wie Firefox, vollständig selbst gezeichnet (keine Kanten oder Grundlinien des Stils): Hintergrund in
/// der Fensterfarbe, der aktive Tab als helle, abgerundete Karte mit feinem Rand und leichtem Schatten unten, die
/// übrigen flach mit kurzen Trennstrichen, beim Darüberfahren getönt. Größe fest: 32 px hoch (Platzsparmodus 30 px).
class RjTabBar : public QTabBar
{
public:
	explicit RjTabBar(QWidget* parent = nullptr)
	  : QTabBar(parent)
	{
		setDrawBase(false);
		setMouseTracking(true);
		setAttribute(Qt::WA_Hover);
		setAutoFillBackground(false);
	}

protected:
	static QColor mix(const QColor& a, const QColor& b, qreal t)
	{
		return QColor::fromRgbF(a.redF() * t + b.redF() * (1 - t), a.greenF() * t + b.greenF() * (1 - t), a.blueF() * t + b.blueF() * (1 - t));
	}
	QSize tabSizeHint(int index) const override
	{
		const int h = property("compact").toBool() ? 30 : 32;
		const int text = fontMetrics().horizontalAdvance(tabText(index).remove(QLatin1Char('&')));
		// Rand 12 + Symbol 16 + Abstand 8 + Text + Abstand 6 + Kreuz samt Rand (gemessen 33 px) + 3 Reserve.
		// Vorher fehlte 1 px, dadurch wurden Titel wie „Neuigkeiten“ in der Mitte gekürzt.
		const int w = qBound(130, 12 + 16 + 8 + text + 6 + 36, 240);
		return QSize(w, h);
	}
	// Bei voller Leiste: kurze Titel bleiben ungekürzt, lange werden höchstens auf 160 px schmaler; danach blättert die
	// Leiste (Pfeile). Vorher schrumpfte jeder Tab auf 120 px („Ne…en“).
	QSize minimumTabSizeHint(int index) const override { return QSize(qMin(160, tabSizeHint(index).width()), tabSizeHint(index).height()); }
	void mouseMoveEvent(QMouseEvent* e) override
	{
		const int hover = tabAt(e->position().toPoint());
		if (hover != m_hover)
		{
			m_hover = hover;
			update();
		}
		QTabBar::mouseMoveEvent(e);
	}
	void leaveEvent(QEvent* e) override
	{
		m_hover = -1;
		update();
		QTabBar::leaveEvent(e);
	}
	void paintEvent(QPaintEvent*) override
	{
		QPainter p(this);
		p.setRenderHint(QPainter::Antialiasing);
		const QPalette pal = palette();
		const QColor win = pal.color(QPalette::Window), text = pal.color(QPalette::WindowText);
		p.fillRect(rect(), win);
		const QColor card = pal.color(QPalette::Base), edge = mix(text, win, 0.16), shadow = mix(text, win, 0.28), sep = mix(text, win, 0.22);
		const int cur = currentIndex();
		for (int i = 0; i < count(); ++i)
		{
			const QRect tr = tabRect(i);
			if (!tr.isValid() || !tr.intersects(rect()))
				continue;
			const QRectF box = QRectF(tr).adjusted(2, 3, -2, -3);
			if (i == cur)
			{
				// Leichter Schatten: dieselbe Form 1 px tiefer in der Schattenfarbe, darüber die Karte.
				p.setPen(Qt::NoPen);
				p.setBrush(shadow);
				p.drawRoundedRect(box.translated(0, 1), 6, 6);
				p.setBrush(card);
				p.setPen(QPen(edge, 1));
				p.drawRoundedRect(box.adjusted(0.5, 0.5, -0.5, -0.5), 6, 6);
			}
			else if (i == m_hover)
			{
				p.setPen(Qt::NoPen);
				p.setBrush(mix(text, win, 0.07));
				p.drawRoundedRect(box, 6, 6);
			}
			else if (i + 1 < count() && i + 1 != cur && i + 1 != m_hover)
			{
				// Kurzer Trennstrich rechts (nicht neben der aktiven oder getönten Karte).
				p.setPen(QPen(sep, 1));
				const qreal x = tr.right() + 0.5;
				p.drawLine(QPointF(x, tr.top() + 9), QPointF(x, tr.bottom() - 8));
			}
			// Symbol und Beschriftung; rechts bleibt Platz für das Schließen-Kreuz (eigenes Bedienelement).
			int x = tr.left() + 12;
			const QIcon icon = tabIcon(i);
			if (!icon.isNull())
			{
				icon.paint(&p, QRect(x, tr.center().y() - 8, 16, 16));
				x += 16 + 8;
			}
			int right = tr.right() - 10;
			if (QWidget* b = tabButton(i, QTabBar::RightSide); b && b->isVisible())
				right = b->geometry().left() - 6;
			const QRect textRect(x, tr.top(), qMax(0, right - x), tr.height());
			p.setPen(i == cur ? text : mix(text, win, 0.85));
			p.drawText(textRect, Qt::AlignVCenter | Qt::AlignLeft, fontMetrics().elidedText(tabText(i).remove(QLatin1Char('&')), Qt::ElideMiddle, textRect.width()));
		}
	}

private:
	int m_hover = -1;
};

/// Tab-Bereich mit der selbst gezeichneten Tableiste.
class RjTabWidget : public QTabWidget
{
public:
	explicit RjTabWidget(QWidget* parent = nullptr)
	  : QTabWidget(parent)
	{
		setTabBar(new RjTabBar(this));
	}

#ifdef Q_OS_WIN
protected:
	// Nur Windows: nur die Fensterfarbe. QTabWidget zeichnete im Dokumentmodus unter die Eckbereiche (Logo,
	// Fensterknöpfe) eine eigene Grundlinie des Stils (graue und weiße Linie). Ubuntu bleibt unverändert.
	void paintEvent(QPaintEvent*) override
	{
		QPainter p(this);
		p.fillRect(rect(), palette().color(QPalette::Window));
	}
#endif
};
} // namespace

RailTab::RailTab(const QString& text, QWidget* parent)
  : QWidget(parent)
  , m_text(text)
{
	setFixedWidth(28);
	setAttribute(Qt::WA_Hover);
	setCursor(Qt::PointingHandCursor);
	setToolTip(text + QStringLiteral(" (F9)"));
	setAccessibleName(text);
}

void RailTab::paintEvent(QPaintEvent*)
{
	QPainter p(this);
	p.setRenderHint(QPainter::Antialiasing);
	const QPalette pal = palette();
	p.fillRect(rect(), pal.color(QPalette::Window));
	// Karte: oben am Rand, so hoch wie Symbol und Text; beim Darüberfahren getönt.
	const QFontMetrics fm(font());
	const int len = fm.horizontalAdvance(m_text) + 34;
	const QRectF card(3.5, 8.5, width() - 7, len);
	p.setPen(QPen(pal.color(QPalette::Mid), 1));
	p.setBrush(underMouse() ? pal.color(QPalette::AlternateBase) : pal.color(QPalette::Base));
	p.drawRoundedRect(card, 6, 6);
	faPaint(&p, QRectF(card.x() + 3, card.y() + 6, card.width() - 6, 14), QStringLiteral("server"), faMutedColor());
	p.save();
	p.translate(card.center().x() + fm.ascent() / 2.0 - 1, card.bottom() - 6);
	p.rotate(-90);
	p.setPen(pal.color(QPalette::WindowText));
	p.drawText(QPointF(0, 0), m_text);
	p.restore();
}

void RailTab::enterEvent(QEnterEvent* e)
{
	QWidget::enterEvent(e);
	update();
	if (onEnter)
		onEnter();
}

void RailTab::leaveEvent(QEvent* e)
{
	QWidget::leaveEvent(e);
	update();
	if (onLeave)
		onLeave();
}

void RailTab::mousePressEvent(QMouseEvent* e)
{
	if (e->button() == Qt::LeftButton && onClick)
		onClick();
	QWidget::mousePressEvent(e);
}

namespace
{
/// Unsichtbare Greifzone am Fensterrand (6 px innen): Zeiger für die Größenänderung, Druck startet sie.
class EdgeGrip : public QWidget
{
public:
	EdgeGrip(QWidget* window, Qt::Edges edges)
	  : QWidget(window)
	  , m_edges(edges)
	{
		setAttribute(Qt::WA_NoSystemBackground);
		setAttribute(Qt::WA_TranslucentBackground);
		const bool diagA = edges == (Qt::LeftEdge | Qt::TopEdge) || edges == (Qt::RightEdge | Qt::BottomEdge);
		const bool diagB = edges == (Qt::RightEdge | Qt::TopEdge) || edges == (Qt::LeftEdge | Qt::BottomEdge);
		setCursor(diagA ? Qt::SizeFDiagCursor : diagB ? Qt::SizeBDiagCursor : (edges & (Qt::LeftEdge | Qt::RightEdge)) ? Qt::SizeHorCursor : Qt::SizeVerCursor);
	}
	Qt::Edges edges() const { return m_edges; }

protected:
	void paintEvent(QPaintEvent*) override {}
	void mousePressEvent(QMouseEvent* e) override
	{
		if (e->button() == Qt::LeftButton && window()->windowHandle())
		{
			window()->windowHandle()->startSystemResize(m_edges);
			e->accept();
			return;
		}
		QWidget::mousePressEvent(e);
	}

private:
	Qt::Edges m_edges;
};
} // namespace

RoundCorners::RoundCorners(QMainWindow* window)
  : QObject(window)
  , m_window(window)
{
	window->installEventFilter(this);
	for (Qt::Edges e : { Qt::Edges(Qt::LeftEdge), Qt::Edges(Qt::RightEdge), Qt::Edges(Qt::TopEdge), Qt::Edges(Qt::BottomEdge),
	                     Qt::LeftEdge | Qt::TopEdge, Qt::RightEdge | Qt::TopEdge, Qt::LeftEdge | Qt::BottomEdge, Qt::RightEdge | Qt::BottomEdge })
	{
		auto* g = new EdgeGrip(window, e);
		g->setObjectName(QStringLiteral("edgeGrip"));
		m_grips.append(g);
	}
	m_timer.setSingleShot(true);
	m_timer.setInterval(0);
	connect(&m_timer, &QTimer::timeout, this, &RoundCorners::update);
}

bool RoundCorners::eventFilter(QObject* o, QEvent* e)
{
	switch (e->type())
	{
	case QEvent::Resize:
	case QEvent::Move:
	case QEvent::Show:
	case QEvent::Hide:
	case QEvent::WindowStateChange:
	case QEvent::ChildAdded:
	case QEvent::LayoutRequest:
		m_timer.start();
		break;
	default:
		break;
	}
	return QObject::eventFilter(o, e);
}

void RoundCorners::update()
{
	// Direkte Fensterteile (Titel- und Menüleiste, Mitte, Statuszeile), die eine Ecke berühren, bekommen dort eine
	// abgerundete Maske; ihre Kinder werden mit beschnitten. Maximiert: keine Masken.
	const bool plain = m_window->isMaximized() || m_window->isFullScreen();
	const QRect inner = m_window->contentsRect();
	const int r = kRadius;
	const QRect corners[4] = { QRect(inner.topLeft(), QSize(r, r)), QRect(inner.right() - r + 1, inner.top(), r, r),
		                       QRect(inner.left(), inner.bottom() - r + 1, r, r), QRect(inner.right() - r + 1, inner.bottom() - r + 1, r, r) };
	const QRect ellipses[4] = { QRect(inner.topLeft(), QSize(2 * r, 2 * r)), QRect(inner.right() - 2 * r + 1, inner.top(), 2 * r, 2 * r),
		                        QRect(inner.left(), inner.bottom() - 2 * r + 1, 2 * r, 2 * r),
		                        QRect(inner.right() - 2 * r + 1, inner.bottom() - 2 * r + 1, 2 * r, 2 * r) };
	// Greifzonen: 6 px innen an Kanten, 14 px in den Ecken; maximiert ausgeblendet; immer obenauf.
	{
		const int g = 6, c = 14, W = m_window->width(), H = m_window->height();
		for (QWidget* grip : std::as_const(m_grips))
		{
			const Qt::Edges e = static_cast<EdgeGrip*>(grip)->edges();
			QRect r;
			if (e == Qt::LeftEdge)
				r = QRect(0, c, g, H - 2 * c);
			else if (e == Qt::RightEdge)
				r = QRect(W - g, c, g, H - 2 * c);
			else if (e == Qt::TopEdge)
				r = QRect(c, 0, W - 2 * c, g);
			else if (e == Qt::BottomEdge)
				r = QRect(c, H - g, W - 2 * c, g);
			else
				r = QRect((e & Qt::LeftEdge) ? 0 : W - c, (e & Qt::TopEdge) ? 0 : H - c, c, c);
			grip->setGeometry(r);
			grip->setVisible(!plain);
			if (!plain)
				grip->raise();
		}
	}
	for (QObject* c : m_window->children())
	{
		auto* w = qobject_cast<QWidget*>(c);
		if (!w || w->isWindow() || w->objectName() == QLatin1String("edgeGrip"))
			continue;
		if (!m_watched.contains(w))
		{
			m_watched.insert(w);
			w->installEventFilter(this);
		}
		if (plain || !w->isVisible())
		{
			if (!w->mask().isEmpty())
				w->clearMask();
			continue;
		}
		QRegion cut;
		for (int i = 0; i < 4; ++i)
			if (w->geometry().intersects(corners[i]))
				cut += QRegion(corners[i]).subtracted(QRegion(ellipses[i], QRegion::Ellipse));
		if (cut.isEmpty())
		{
			if (!w->mask().isEmpty())
				w->clearMask();
			continue;
		}
		const QRegion mask = QRegion(w->rect()).subtracted(cut.translated(-w->pos()));
		if (w->mask() != mask)
			w->setMask(mask);
	}
	m_window->update();
}

MainWindow::MainWindow(RjBackend* backend, QWidget* parent)
  : QMainWindow(parent)
  , m_backend(backend)
{
	// Erweiterungen melden ihre Menüeinträge und ihr Klickverhalten an (vor dem Aufbau der Menüs); der Lader
	// liefert nur freigeschaltete (Lizenz bzw. Vorführung).
	for (RjPluginUi* p : rjPluginUis(m_backend))
		p->attach(this);
	const QString version = m_backend->appVersion();
	setWindowTitle(QStringLiteral("%1 V %2").arg(m_backend->appEdition(), version));
	resize(1280, 800);
	// Eigene Titelleiste (links Logo, Name, Version): gleiches Aussehen unter Ubuntu und Windows.
	setWindowFlag(Qt::FramelessWindowHint, true);
	setMouseTracking(true);
	// Runde Fensterecken (Darstellung, Vorgabe an): Windows 11 rundet über DWM (WinFrame), Linux zeichnet selbst
	// (durchsichtiger Rand, Ecken der Fensterteile ausgespart). Umschalten wirkt nach einem Neustart.
	{
		const bool round = parseObject(m_backend->uiPrefsJson()).value(QStringLiteral("round_corners")).toBool(true);
		setProperty("rjRoundCorners", round);
#ifndef Q_OS_WIN
		if (round)
		{
			setAttribute(Qt::WA_TranslucentBackground);
			m_round = true;
			m_roundCorners = new RoundCorners(this);
		}
#endif
	}
	// Runde Ecken (Linux): kein Rand – das Fenster sitzt bündig am Bildschirm; Größe ziehen über Greifzonen innen.
	if (m_round)
		setContentsMargins(0, 0, 0, 0);
	else
		setContentsMargins(kResizeBorder, kResizeBorder, kResizeBorder, kResizeBorder);
#ifdef Q_OS_WIN
	// Windows: nativer Größenrahmen (Zeiger, Ziehen, Aero Snap); der Innenrand folgt Rahmenbreite und Zustand.
	new WinFrame(this, [this](int m) { setContentsMargins(m, m, m, m); });
#endif
	{
		auto* top = new QWidget(this);
		auto* tl = new QVBoxLayout(top);
		tl->setContentsMargins(0, 0, 0, 0);
		tl->setSpacing(0);
		m_titleBar = new TitleBar(this, m_backend->appEdition(), version, top);
		m_menuBar = new QMenuBar(top);
		tl->addWidget(m_titleBar);
		tl->addWidget(m_menuBar);
		setMenuWidget(top);
	}

	auto* left = new QWidget(this);
	m_leftPane = left;
	auto* leftOuter = new QVBoxLayout(left);
	leftOuter->setContentsMargins(0, 0, 0, 0);
	leftOuter->setSpacing(0);
	// Im Platzsparmodus mit angepinntem Baum steht hier Logo, Name und Version über dem Baum.
	m_brandSlot = new QWidget(left);
	m_brandSlotLay = new QVBoxLayout(m_brandSlot);
	m_brandSlotLay->setContentsMargins(0, 0, 0, 0);
	m_brandSlot->hide();
	m_brandSlot->setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Fixed);
	auto* leftInner = new QWidget(left);
	m_leftInner = leftInner;
	leftOuter->addWidget(m_brandSlot);
	leftOuter->addWidget(leftInner, 1);
	leftOuter->addStretch(0);
	auto* leftLay = new QVBoxLayout(leftInner);
	leftLay->setContentsMargins(4, 4, 4, 4);
	m_search = new QLineEdit(leftInner);
	// Strg+Z im Suchfeld: ohne eigene Eingabe zum Rückgängigmachen gilt es der letzten Änderung am Baum.
	m_search->installEventFilter(this);
	m_search->setPlaceholderText(T(m_backend, "search.placeholder"));
	m_search->setClearButtonEnabled(true);
	m_search->setMinimumHeight(32);
	m_search->setAccessibleName(T(m_backend, "search.placeholder"));
	m_viewBox = new QComboBox(leftInner);
	m_viewBox->setAccessibleName(T(m_backend, "view.label"));
	m_bar = new ActionBar(m_backend, leftInner);
	auto* tree = new RjTree(leftInner);
	m_tree = tree;
	tree->setDragEnabled(true);
	tree->setAcceptDrops(true);
	tree->setDropIndicatorShown(true);
	tree->setDragDropMode(QAbstractItemView::InternalMove);
	tree->onDrop = [this](const QModelIndex& src, const QModelIndex& dst, int pos) {
		if (src.data(RoleGuest).isValid() || src.data(RolePlaceholder).toBool() || dst.data(RoleGuest).isValid() || dst.data(RolePlaceholder).toBool() ||
		    src.data(RoleMacro).toInt() || dst.data(RoleMacro).toInt())
			return;
		QString parent;
		QJsonValue at;
		if (!dst.isValid() || pos == 3)
			parent = QString();
		else if (pos == 0 && dst.data(RoleFolder).toBool())
			parent = dst.data(RoleId).toString();
		else
		{
			parent = dst.parent().isValid() ? dst.parent().data(RoleId).toString() : QString();
			at = dst.row() + (pos == 2 ? 1 : 0);
		}
		QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("move") }, { QStringLiteral("id"), src.data(RoleId).toString() } };
		if (!parent.isEmpty())
			cmd.insert(QStringLiteral("parent"), parent);
		if (!at.isUndefined())
			cmd.insert(QStringLiteral("pos"), at);
		const QString id = src.data(RoleId).toString();
		QTimer::singleShot(0, this, [this, cmd, id] {
			showEditResult(parseObject(edit(cmd)));
			selectId(id);
		});
	};
	m_model = new QStandardItemModel(this);
	m_tree->setModel(m_model);
	m_tree->setHeaderHidden(true);
	// Doppelklick verbindet; Ordner klappt der Handler selbst auf. Hypervisor-Gäste lädt nur der Pfeil.
	m_tree->setExpandsOnDoubleClick(false);
	m_tree->setUniformRowHeights(true);
	m_tree->setIconSize(QSize(20, 20));
	m_tree->setContextMenuPolicy(Qt::CustomContextMenu);
	m_tree->setSelectionMode(QAbstractItemView::ExtendedSelection);
	// Hinweis im leeren Baum: sagt, wie man ein erstes Ziel oder einen Ordner anlegt.
	m_emptyHint = new QWidget(m_tree->viewport());
	{
		auto* el = new QVBoxLayout(m_emptyHint);
		el->setContentsMargins(12, 12, 12, 12);
		el->addStretch(1);
		m_emptyLabel = new QLabel(T(m_backend, "tree.empty"), m_emptyHint);
		m_emptyLabel->setWordWrap(true);
		m_emptyLabel->setAlignment(Qt::AlignCenter);
		el->addWidget(m_emptyLabel);
		m_emptyButton = new QPushButton(T(m_backend, "tree.empty.add"), m_emptyHint);
		el->addWidget(m_emptyButton, 0, Qt::AlignHCenter);
		el->addStretch(2);
		// Ohne entsperrte Schlüsseldatenbank führt der Knopf zum Entsperren, sonst zum Hinzufügen.
		connect(m_emptyButton, &QPushButton::clicked, this, [this] {
			if (m_backend->isVaultOpen())
				popupAddMenu();
			else
				showUnlock(false);
		});
		m_emptyHint->hide();
		m_tree->viewport()->installEventFilter(this);
	}
	m_tree->setAccessibleName(QStringLiteral("Verbindungsbaum"));
	// Kopfzeile „Ziele“ mit Stecknadel (angeheftet bzw. automatisch einklappen), darunter Leiste und Baum, unten die
	// Suche mit grauer Lupe. Die frühere Ansichtenauswahl entfällt (Favoriten stehen als Ordner im Baum).
	{
		auto* header = new QWidget(leftInner);
		header->setObjectName(QStringLiteral("treeHeader"));
		auto* hl = new QHBoxLayout(header);
		hl->setContentsMargins(6, 0, 0, 0);
		hl->setSpacing(4);
		auto* title = new QLabel(T(m_backend, "tree.header"), header);
		QFont hf = title->font();
		hf.setWeight(QFont::DemiBold);
		title->setFont(hf);
		m_pinButton = new QToolButton(header);
		m_pinButton->setAutoRaise(true);
		m_pinButton->setFocusPolicy(Qt::NoFocus);
		m_pinButton->setFixedSize(28, 24);
		connect(m_pinButton, &QToolButton::clicked, this, [this] { setPinned(!m_pinned, true); });
		// Alle zuklappen: klappt den ganzen Baum zu; auch nach einer Suche bleibt er danach zu.
		auto* collapse = new QToolButton(header);
		collapse->setObjectName(QStringLiteral("treeCollapseAll"));
		collapse->setAutoRaise(true);
		collapse->setFocusPolicy(Qt::NoFocus);
		collapse->setFixedSize(28, 24);
		collapse->setIcon(faIcon(QStringLiteral("r-square-minus")));
		collapse->setToolTip(T(m_backend, "tree.collapse.all"));
		collapse->setAccessibleName(T(m_backend, "tree.collapse.all"));
		connect(collapse, &QToolButton::clicked, this, [this] {
			m_tree->collapseAll();
			m_expandedIds.clear();
		});
		hl->addWidget(title);
		hl->addStretch(1);
		hl->addWidget(collapse);
		hl->addWidget(m_pinButton);
		leftLay->addWidget(header);
	}
	m_viewBox->hide();
	m_search->addAction(faIcon(QStringLiteral("magnifying-glass")), QLineEdit::LeadingPosition);
	leftLay->addWidget(m_bar);
	leftLay->addWidget(m_tree, 1);
	leftLay->addWidget(m_search);

	m_tabs = new RjTabWidget(this);
	m_tabs->setTabsClosable(true);
	m_tabs->setMovable(true);
	m_tabs->setDocumentMode(true);
	m_tabs->tabBar()->setContextMenuPolicy(Qt::CustomContextMenu);
	configureTabBar(m_tabs);

	auto* split = new QSplitter(Qt::Horizontal, this);
	split->addWidget(left);
	split->addWidget(m_tabs);
	split->setStretchFactor(1, 1);
	split->setSizes({ 300, 980 });
	// Breiterer Trenner zwischen Baum und Tabs: leichter zu treffen, beim Darüberfahren hervorgehoben.
	split->setHandleWidth(7);
	m_split = split;
	// Mitte: links die Registerkarte „Ziele“ (nur bei gelöster Stecknadel), daneben Baum und Tabs.
	m_center = new QWidget(this);
	{
		auto* cl = new QHBoxLayout(m_center);
		cl->setContentsMargins(0, 0, 0, 0);
		cl->setSpacing(0);
		m_rail = new RailTab(T(m_backend, "tree.header"), m_center);
		m_rail->onEnter = [this] {
			if (!m_flyOpen)
				m_flyOpenTimer.start(m_flyOpenMs);
			m_flyCloseTimer.stop();
		};
		m_rail->onLeave = [this] {
			m_flyOpenTimer.stop();
			if (m_flyOpen)
				m_flyCloseTimer.start(m_flyCloseMs);
		};
		m_rail->onClick = [this] { m_flyOpen ? closeFlyout() : openFlyout(); };
		m_rail->hide();
		cl->addWidget(m_rail);
		cl->addWidget(split, 1);
	}
	m_center->installEventFilter(this);
	m_flyOpenTimer.setSingleShot(true);
	m_flyCloseTimer.setSingleShot(true);
	connect(&m_flyOpenTimer, &QTimer::timeout, this, [this] { openFlyout(); });
	connect(&m_flyCloseTimer, &QTimer::timeout, this, [this] {
		if (!m_flyOpen)
			return;
		// Offenes Kontextmenü oder Maus über dem Bereich: später noch einmal prüfen.
		if (QApplication::activePopupWidget() || m_leftPane->underMouse() || m_rail->underMouse())
		{
			m_flyCloseTimer.start(m_flyCloseMs);
			return;
		}
		closeFlyout();
	});
	connect(qApp, &QApplication::focusChanged, this, [this](QWidget*, QWidget* now) {
		if (!m_pinned && m_flyOpen && now && !m_leftPane->isAncestorOf(now) && now != m_leftPane && !QApplication::activePopupWidget())
			m_flyCloseTimer.start(m_flyCloseMs);
	});
	setCentralWidget(m_center);

	buildMenus();
	// Hamburger-Menü (Platzsparmodus): dieselben Menüs; die Kürzel gelten auch bei verborgener Menüzeile.
	applyMenuIcons();
	// Hamburger-Menü: die Menüs Datei, Bearbeiten, Extras, Ansicht, Hilfe als Untermenüs (mit Symbolen).
	m_burgerMenu = new QMenu(this);
	for (QAction* a : m_menuBar->actions())
		if (a->menu())
			m_burgerMenu->addMenu(a->menu());
	{
		std::function<void(QMenu*)> carry = [&](QMenu* m) {
			for (QAction* a : m->actions())
			{
				if (a->menu())
					carry(a->menu());
				else if (!a->shortcut().isEmpty())
					addAction(a);
			}
		};
		for (QAction* a : m_menuBar->actions())
			if (a->menu())
				carry(a->menu());
	}
	m_tabs->tabBar()->installEventFilter(this);
	m_tabs->installEventFilter(this);
	{
		// Logo unten rechts, solange kein Tab offen ist (halbtransparent, Klicks gehen durch).
		const QPixmap src(QStringLiteral(":/logo/remotejoe-512.png"));
		const qreal dpr = devicePixelRatioF();
		QPixmap pm(QSize(200, 200) * dpr);
		pm.setDevicePixelRatio(dpr);
		pm.fill(Qt::transparent);
		{
			QPainter p(&pm);
			p.setRenderHint(QPainter::SmoothPixmapTransform);
			p.setOpacity(0.35);
			p.drawPixmap(QRect(0, 0, 200, 200), src);
		}
		m_logo = new QLabel(m_tabs);
		m_logo->setObjectName(QStringLiteral("emptyLogo"));
		m_logo->setPixmap(pm);
		m_logo->setFixedSize(200, 200);
		// Das Logo selbst ist der Link auf die Projektseite (öffnet im Standardbrowser); kein Text darunter.
		m_logo->setCursor(Qt::PointingHandCursor);
		m_logo->setToolTip(QStringLiteral("github.com/joeMJ/remotejoe"));
		m_logo->setAccessibleName(T(m_backend, "logo.link"));
		m_logo->installEventFilter(this);
		connect(m_tabs, &QTabWidget::currentChanged, this, [this] { updateLogo(); });
		QTimer::singleShot(0, this, [this] { updateLogo(); });
	}
	// Regelmäßige Update-Suche (Einstellungen → Updates): alle zehn Minuten nachsehen, ob sie fällig ist.
	{
		auto* periodic = new QTimer(this);
		periodic->setInterval(10 * 60 * 1000);
		connect(periodic, &QTimer::timeout, this, [this] {
			const QJsonObject s = parseObject(m_backend->edit(QStringLiteral("{\"op\":\"update_status\"}")));
			if (!s.value(QStringLiteral("periodic")).toBool() || s.value(QStringLiteral("busy")).toBool() || m_backend->isDemo())
				return;
			const QString unit = s.value(QStringLiteral("unit")).toString();
			const qint64 step = unit == QLatin1String("stunden") ? 3600 : unit == QLatin1String("wochen") ? 7 * 86400 : 86400;
			const qint64 every = qMax(1, s.value(QStringLiteral("every")).toInt(1)) * step;
			const qint64 last = static_cast<qint64>(s.value(QStringLiteral("last_check")).toDouble());
			const qint64 now = static_cast<qint64>(s.value(QStringLiteral("now")).toDouble());
			if (now - last >= every)
				m_backend->edit(QStringLiteral("{\"op\":\"update_check\"}"));
		});
		periodic->start();
	}
	m_fitTimer.setSingleShot(true);
	m_fitTimer.setInterval(300);
	connect(&m_fitTimer, &QTimer::timeout, this, &MainWindow::fitToScreen);
	connect(qApp, &QGuiApplication::screenAdded, this, [this] { screenLayoutChanged(); });
	connect(qApp, &QGuiApplication::screenRemoved, this, [this] { screenLayoutChanged(); });
	connect(qApp, &QGuiApplication::primaryScreenChanged, this, [this] { screenLayoutChanged(); });
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
	// Hell/Dunkel des Desktops live übernehmen (ab Qt 6.5; unter Ubuntu 24.04 mit Qt 6.4 gilt es beim Start).
	connect(QGuiApplication::styleHints(), &QStyleHints::colorSchemeChanged, this, [this] { applyAppearance(); });
#endif
	FrameView::setFocusFrame(parseObject(m_backend->uiPrefsJson()).value(QStringLiteral("focus_frame")).toBool(true));
	{
		const QJsonObject st = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("settings") } }));
		FrameView::s_keyCapture = st.value(QStringLiteral("keyboard_capture")).toBool(true);
		m_closeTabKey = st.value(QStringLiteral("close_tab_key")).toString(QStringLiteral("outside"));
		m_findKey = st.value(QStringLiteral("find_key")).toString(QStringLiteral("outside"));
	}
	FrameView::s_notice = [this](const QString& m) { statusBar()->showMessage(m, 8000); };
	FrameView::s_adminTimeout = [this](FrameView* f) {
		for (SessionPane* p : std::as_const(m_panes))
			if (p->frame() == f)
				setAdminKeys(p, false, T(m_backend, "adminkeys.off.idle"));
	};
	statusBar()->showMessage(T(m_backend, "status.ready"));
	// Fußzeile rechts: womit, wohin, seit wann, als wer.
	m_connInfo = new QWidget(this);
	{
		auto* l = new QHBoxLayout(m_connInfo);
		l->setContentsMargins(0, 0, 4, 0);
		l->setSpacing(3);
		auto add = [&](QLabel*& icon, QLabel*& text, const char* tip) {
			icon = new QLabel(m_connInfo);
			icon->setFixedSize(11, 11);
			icon->setScaledContents(true);
			text = new QLabel(m_connInfo);
			text->setToolTip(T(m_backend, tip));
			icon->setToolTip(T(m_backend, tip));
			l->addWidget(icon);
			l->addWidget(text);
			l->addSpacing(6);
		};
		add(m_ciProtoIcon, m_ciProto, "ci.tip.type");
		add(m_ciTargetIcon, m_ciTarget, "ci.tip.target");
		add(m_ciSinceIcon, m_ciSince, "ci.tip.since");
		add(m_ciUserIcon, m_ciUser, "ci.tip.user");
		m_ciTarget->setTextInteractionFlags(Qt::TextSelectableByMouse);
	}
	statusBar()->addPermanentWidget(m_connInfo);
	m_connInfo->hide();
	connect(&m_ciTimer, &QTimer::timeout, this, &MainWindow::updateConnInfo);
	m_ciTimer.start(1000);

	// Eingabetaste und Doppelklick verbinden; die Eigenschaften öffnen F4, Alt+Eingabe und das Kontextmenü.
	auto treeKey = [this, left](const QKeySequence& k, auto fn) {
		auto* s = new QShortcut(k, left);
		s->setContext(Qt::WidgetWithChildrenShortcut);
		connect(s, &QShortcut::activated, this, fn);
	};
	auto activate = [this] {
		const QModelIndex i = m_tree->currentIndex();
		if (!m_tree->hasFocus() || !i.isValid())
			return;
		if (i.data(RoleMacro).toInt() == 2)
		{
			openMacroTab(i.data(RoleId).toString());
			return;
		}
		if (i.data(RoleMacro).toInt() == 1)
		{
			m_tree->setExpanded(i, !m_tree->isExpanded(i));
			return;
		}
		if (i.data(RoleGuest).isValid())
			openGuestConsole(i);
		else if (!i.data(RoleFolder).toBool())
			connectSelected();
		else
			m_tree->setExpanded(i, !m_tree->isExpanded(i));
	};
	treeKey(QKeySequence(Qt::Key_Return), activate);
	treeKey(QKeySequence(Qt::Key_Enter), activate);
	connect(m_tree, &QTreeView::expanded, this, [this](const QModelIndex& i) {
		if (i.data(RoleMacro).toInt() == 1)
			m_macroShelfOpen = true;
		if (i.data(RoleFavFolder).toBool() && !m_autoExpand)
			m_favOpen = true;
	});
	connect(m_tree, &QTreeView::collapsed, this, [this](const QModelIndex& i) {
		if (i.data(RoleMacro).toInt() == 1)
			m_macroShelfOpen = false;
		if (i.data(RoleFavFolder).toBool() && !m_autoExpand)
			m_favOpen = false;
	});
	// Einfachklick auf ein Ziel: Objektübersicht (Doppelklick verbindet wie bisher).
	connect(m_tree, &QTreeView::clicked, this, [this](const QModelIndex& i) {
		if (QGuiApplication::keyboardModifiers() & (Qt::ControlModifier | Qt::ShiftModifier))
			return; // Mehrfachauswahl
		for (const auto& h : m_modules.clickTop)
			if (h(i))
				return;
		if (i.data(RoleGuest).isValid())
		{
			openOverview(QJsonObject{ { QStringLiteral("hv"), i.data(RoleGuestTarget).toString() },
			                          { QStringLiteral("guest"), i.data(RoleGuest).toString() },
			                          { QStringLiteral("name"), i.data(Qt::DisplayRole).toString() },
			                          { QStringLiteral("state"), i.data(RoleGuestState).toString() },
			                          { QStringLiteral("address"), i.data(RoleConsoleHost).toString() } },
			             i.data(Qt::DecorationRole).value<QIcon>());
			return;
		}
		if (i.data(RoleLogin).toBool())
		{
			openLogin(i.data(RoleId).toString(), QString()); // der Anmeldungs-Tab ist ihre Übersicht
			return;
		}
		if (i.data(RoleFolder).toBool() && i.data(RoleMacro).toInt() == 0 && !i.data(RolePlaceholder).toBool())
		{
			// Module zuerst, sonst die Ordnerübersicht.
			for (const auto& h : m_modules.clickFolder)
				if (h(i))
					return;
			openFolderOverview(i);
			return;
		}
		if (i.data(RoleMacro).toInt() != 0 || i.data(RolePlaceholder).toBool() || i.data(RoleFolder).toBool())
			return;
		const QString id = i.data(RoleId).toString();
		if (!id.isEmpty())
			openOverview(QJsonObject{ { QStringLiteral("id"), id } }, i.data(Qt::DecorationRole).value<QIcon>());
	});
	connect(m_tree, &QTreeView::doubleClicked, this, [this](const QModelIndex& i) {
		if (i.data(RoleMacro).toInt() == 2)
		{
			openMacroTab(i.data(RoleId).toString());
			return;
		}
		if (i.data(RoleGuest).isValid())
			openGuestConsole(i);
		else if (!i.data(RolePlaceholder).toBool() && !i.data(RoleFolder).toBool())
		{
			connectSelected();
			// Hypervisor mit Hauptweg für den Doppelklick: Gäste wie bisher aufklappen (und laden). Öffnet der
			// Doppelklick einen anderen Weg (z. B. RDP), lädt nur der Pfeil die Gäste.
			const QModelIndex first = m_model->index(0, 0, i);
			if (first.isValid() && first.data(RolePlaceholder).toBool() && !i.data(RoleOtherWay).toBool())
				m_tree->expand(i);
		}
		else if (i.data(RoleFolder).toBool() || i.data(RoleMacro).toInt() == 1)
			m_tree->setExpanded(i, !m_tree->isExpanded(i)); // Ordner klappen wie üblich auf oder zu.
	});
	treeKey(QKeySequence(Qt::Key_F4), [this] { if (!currentId().isEmpty()) openProperties(currentId()); });
	treeKey(QKeySequence(QStringLiteral("Alt+Return")), [this] { if (!currentId().isEmpty()) openProperties(currentId()); });
	treeKey(QKeySequence(QStringLiteral("Ctrl+N")), [this] { popupAddMenu(); });
	treeKey(QKeySequence(Qt::Key_Insert), [this] { popupAddMenu(); });
	treeKey(QKeySequence(Qt::Key_F2), [this] { renameSelected(); });
	treeKey(QKeySequence(Qt::Key_Delete), [this] {
		const QModelIndex i = m_tree->currentIndex();
		if (i.isValid() && i.data(RoleMacro).toInt() == 2)
			deleteMacro(i.data(RoleId).toString(), i.data(Qt::DisplayRole).toString());
		else if (i.isValid() && i.data(RoleFavFolder).toBool())
			return;
		else if (i.isValid() && i.data(RoleFavLink).toBool())
			toggleFavorite(); // Entf im Ordner „Favoriten“ nimmt nur den Favoriten heraus.
		else
			deleteSelected();
	});
	treeKey(QKeySequence(QStringLiteral("Ctrl+D")), [this] { duplicateSelected(); });
	treeKey(QKeySequence(QStringLiteral("Ctrl+Shift+M")), [this] { moveSelected(); });
	treeKey(QKeySequence(QStringLiteral("Alt+Up")), [this] { reorderSelected(-1); });
	treeKey(QKeySequence(QStringLiteral("Alt+Down")), [this] { reorderSelected(1); });
	treeKey(QKeySequence(QStringLiteral("Ctrl+B")), [this] { toggleFavorite(); });
	treeKey(QKeySequence(QStringLiteral("Ctrl+Z")), [this] { undo(); });
	connect(m_viewBox, &QComboBox::activated, this, [this](int) {
		edit({ { QStringLiteral("op"), QStringLiteral("set_view") }, { QStringLiteral("view"), m_viewBox->currentData().toString() } });
		reloadTree();
	});
	m_overlay = new UnlockOverlay(m_backend, this);
	connect(m_overlay, &UnlockOverlay::unlockRequested, this, [this](const QString& path, const QString& pw, const QString& keyFile, bool remember) {
		m_backend->unlock(path, pw, keyFile, remember);
		statusBar()->showMessage(T(m_backend, "status.unlocking"));
	});
	connect(m_overlay, &UnlockOverlay::createRequested, this, [this](const QString& path, const QString& pw, const QString& keyFile, bool makeKey, bool remember) {
		m_backend->createVault(path, pw, keyFile, makeKey, remember);
		statusBar()->showMessage(T(m_backend, "status.creating"));
	});
	// Hintergrundaufgaben in der Statuszeile: Fortschritt und Abbruch.
	m_jobsBox = new QWidget(this);
	auto* jl = new QHBoxLayout(m_jobsBox);
	jl->setContentsMargins(0, 0, 0, 0);
	auto* busy = new QProgressBar(m_jobsBox);
	busy->setRange(0, 0);
	busy->setMaximumWidth(120);
	busy->setMaximumHeight(8);
	m_jobsLabel = new QLabel(m_jobsBox);
	auto* cancelJob = new QPushButton(T(m_backend, "jobs.cancel"), m_jobsBox);
	cancelJob->setObjectName(QStringLiteral("cancelJob"));
	connect(cancelJob, &QPushButton::clicked, this, [this] {
		const QJsonArray jobs = QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("jobs") } }).toUtf8()).array();
		if (!jobs.isEmpty())
			edit({ { QStringLiteral("op"), QStringLiteral("cancel_job") }, { QStringLiteral("job"), jobs.first().toObject().value(QStringLiteral("id")) } });
		updateJobs();
	});
	jl->addWidget(busy);
	jl->addWidget(m_jobsLabel);
	jl->addWidget(cancelJob);
	statusBar()->addPermanentWidget(m_jobsBox);
	m_jobsBox->hide();
	// Fußzeile (kompakt): Baum-Knopf, Hinweise der vorderen Sitzung in der Mitte, rechts der Schalter.
	statusBar()->setSizeGripEnabled(false);
	{
		// Fußzeile nur halb so hoch wie üblich: kleinere Schrift, keine Ränder.
		QFont sf = statusBar()->font();
		sf.setPointSizeF(sf.pointSizeF() * 0.8);
		statusBar()->setFont(sf);
		statusBar()->setContentsMargins(0, 0, 0, 0);
	}
	m_treeButton = new QToolButton(this);
	m_treeButton->setAutoRaise(true);
	m_treeButton->setFocusPolicy(Qt::NoFocus);
	m_treeButton->setObjectName(QStringLiteral("statusTreeButton"));
	connect(m_treeButton, &QToolButton::clicked, this, [this] { setTreeVisible(m_treeHidden); });
	statusBar()->insertPermanentWidget(0, m_treeButton);
	m_noticeLabel = new QLabel(this);
	m_noticeLabel->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
	m_noticeLabel->setContentsMargins(8, 0, 8, 0);
	m_noticeLabel->setObjectName(QStringLiteral("statusNotice"));
	m_noticeLabel->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
	statusBar()->insertPermanentWidget(1, m_noticeLabel, 1);
	setTreeVisible(true);
	// Leerlauf: Eingaben überall im Programm zählen.
	m_activityClock.start();
	qApp->installEventFilter(this);
#ifdef Q_OS_LINUX
	QDBusConnection::systemBus().connect(QStringLiteral("org.freedesktop.login1"), QStringLiteral("/org/freedesktop/login1"),
	                                     QStringLiteral("org.freedesktop.login1.Manager"), QStringLiteral("PrepareForSleep"), this,
	                                     SLOT(onPrepareForSleep(bool)));
#endif
#ifdef Q_OS_WIN
	static PowerFilter power([this] { systemResumed(); });
	qApp->installNativeEventFilter(&power);
#endif
	connect(m_tree, &QTreeView::expanded, this, [this](const QModelIndex& i) {
		if (m_autoExpand)
			return;
		const QModelIndex first = m_model->index(0, 0, i);
		if (first.isValid() && first.data(RolePlaceholder).toBool())
			loadGuests(i);
	});
	connect(m_tree, &QTreeView::customContextMenuRequested, this, &MainWindow::treeContextMenu);
	m_filterTimer.setSingleShot(true);
	m_filterTimer.setInterval(150);
	connect(m_search, &QLineEdit::textChanged, this, [this] { m_filterTimer.start(); });
	connect(&m_filterTimer, &QTimer::timeout, this, &MainWindow::applySearch);
	connect(m_tabs, &QTabWidget::tabCloseRequested, this, [this](int i) {
		if (auto* st = qobject_cast<SnmpTab*>(m_tabs->widget(i)))
		{
			st->wipeSecrets();
			m_tabs->removeTab(i);
			st->deleteLater();
			return;
		}
		if (auto* tt = qobject_cast<ToolTab*>(m_tabs->widget(i)))
		{
			closeToolTab(tt);
			return;
		}
		// Tab einer Erweiterung (RjPluginTab in deren Bibliothek): wie ihr Knopf „Schließen“.
		if (QWidget* pw = m_tabs->widget(i); pw && pw->inherits("RjPluginTab"))
		{
			// Wie ihr Knopf „Schließen“ (Signal closeRequested); ohne das Signal direkt schließen.
			if (pw->metaObject()->indexOfSignal("closeRequested()") >= 0)
				QMetaObject::invokeMethod(pw, "closeRequested", Qt::QueuedConnection);
			else
				closeToolTab(pw);
			return;
		}
		if (auto* mt = qobject_cast<MacroTab*>(m_tabs->widget(i)))
		{
			if (mt->isDirty())
			{
				m_tabs->setCurrentIndex(i);
				statusBar()->showMessage(T(m_backend, "macro.close.dirty"), 8000);
				return;
			}
			m_tabs->removeTab(i);
			mt->deleteLater();
			return;
		}
		if (m_tabs->widget(i)->objectName() == QLatin1String("newsTab"))
		{
			QWidget* news = m_tabs->widget(i);
			m_tabs->removeTab(i);
			news->deleteLater();
			return;
		}
		if (auto* ts = qobject_cast<TerminalSettingsTab*>(m_tabs->widget(i)))
		{
			m_tabs->removeTab(i);
			ts->deleteLater();
			return;
		}
		if (auto* ed = qobject_cast<PropertiesTab*>(m_tabs->widget(i)))
		{
			m_tabs->setCurrentWidget(ed);
			ed->requestClose([this, ed] {
				m_tabs->removeTab(m_tabs->indexOf(ed));
				ed->deleteLater();
			});
			return;
		}
		const auto tab = m_tabs->widget(i)->property("tab").toULongLong();
		for (const QJsonValue& t : m_workspace.value(QStringLiteral("tabs")).toArray())
			if (std::uint64_t(t.toObject().value(QStringLiteral("id")).toInteger()) == tab)
				for (const QJsonValue& s : t.toObject().value(QStringLiteral("sessions")).toArray())
				{
					const auto sid = std::uint64_t(s.toInteger());
					m_backend->closeSession(sid);
					if (auto* p = m_panes.take(sid))
						p->deleteLater();
				}
		rebuildTabs();
	});
	connect(m_tabs, &QTabWidget::currentChanged, this, [this](int) {
		updateNotice();
		if (auto t = currentTab())
			m_backend->activateTab(t);
		updateStatus();
	});
	connect(m_tabs->tabBar(), &QTabBar::customContextMenuRequested, this,
	        [this](const QPoint& p) { tabContextMenu(m_tabs, p); });

	// Tastaturbedienung.
	auto sc = [this](const QKeySequence& k, auto fn) {
		auto* s = new QShortcut(k, this);
		s->setContext(Qt::ApplicationShortcut);
		connect(s, &QShortcut::activated, this, fn);
	};
	sc(QKeySequence(QStringLiteral("Ctrl+F")), [this] {
		// Fokus in einer Web-Sitzung: in der Seite suchen; sonst die Suche über dem Baum (wenn eingestellt).
		if (SessionPane* pane = focusedWebPane())
		{
			pane->openFind();
			return;
		}
		if (!findKeyActive())
			return;
		if (!m_pinned)
			openFlyout();
		m_search->setFocus();
		m_search->selectAll();
	});
	sc(QKeySequence(QStringLiteral("Ctrl+Alt+Home")), [this] { m_tree->setFocus(); });
	sc(QKeySequence(QStringLiteral("Ctrl+Shift+W")), [this] {
		if (closeTabKeyActive() && m_tabs->count() > 0)
			emit m_tabs->tabCloseRequested(m_tabs->currentIndex());
	});
	sc(QKeySequence(QStringLiteral("Ctrl+PgDown")), [this] { stepTab(1); });
	sc(QKeySequence(QStringLiteral("Ctrl+PgUp")), [this] { stepTab(-1); });
	// Tab-Wechsel mit frei belegbaren Tastenfolgen (Einstellungen → Tastatur), Vorgabe Strg+Umschalt+Bild↑/↓.
	m_tabPrevShortcut = new QShortcut(this);
	m_tabPrevShortcut->setContext(Qt::ApplicationShortcut);
	connect(m_tabPrevShortcut, &QShortcut::activated, this, [this] {
		if (tabSwitchKeyActive())
			stepTab(-1);
	});
	m_tabNextShortcut = new QShortcut(this);
	m_tabNextShortcut->setContext(Qt::ApplicationShortcut);
	connect(m_tabNextShortcut, &QShortcut::activated, this, [this] {
		if (tabSwitchKeyActive())
			stepTab(1);
	});
	{
		const QJsonObject st = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("settings") } }));
		applyTabSwitchKeys(st.value(QStringLiteral("tab_switch_key")).toString(QStringLiteral("outside")), st.value(QStringLiteral("tab_prev_keys")).toString(QStringLiteral("Ctrl+Shift+PgUp")), st.value(QStringLiteral("tab_next_keys")).toString(QStringLiteral("Ctrl+Shift+PgDown")));
	}
	connect(m_search, &QLineEdit::returnPressed, this, [this] { m_tree->setFocus(); m_tree->setCurrentIndex(m_model->index(0, 0)); });

	m_poll.setInterval(16);
	connect(&m_poll, &QTimer::timeout, this, &MainWindow::poll);
	m_poll.start();
	reloadViews();
	reloadTree();
	rebuildTabs();
#ifdef Q_OS_WIN
	// Ohne WebView2-Laufzeit gehen Web-Ziele nicht: beim Start einmal deutlich darauf hinweisen (mit Link).
	if (qEnvironmentVariableIsEmpty("RJ_DEMO") && qEnvironmentVariableIsEmpty("RJ_SCREENSHOTS") && !webView2Installed())
		QTimer::singleShot(1500, this, [this] {
			auto* box = new QMessageBox(QMessageBox::Information, QStringLiteral("remotejoe"), T(m_backend, "webview2.missing"), QMessageBox::Ok, this);
			box->setTextFormat(Qt::RichText);
			box->setTextInteractionFlags(Qt::TextBrowserInteraction);
			box->setAttribute(Qt::WA_DeleteOnClose);
			box->setWindowModality(Qt::NonModal);
			box->show();
		});
	// Neustart nach einem Update über die MSI: Ergebnis von msiexec (0 bzw. 3010: erfolgreich) in der Fußzeile.
	if (const QString code = qEnvironmentVariable("RJ_UPDATE_RESULT"); !code.isEmpty())
	{
		qunsetenv("RJ_UPDATE_RESULT");
		const QString log = qEnvironmentVariable("RJ_UPDATE_LOG");
		qunsetenv("RJ_UPDATE_LOG");
		// Verständlicher Text mit Handlungshinweis und Protokoll (msi.rs).
		const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("update_msi_result") }, { QStringLiteral("code"), code }, { QStringLiteral("log"), log } }));
		g_updateNotice = r.value(QStringLiteral("ok")).toBool() ? T(m_backend, "update.applied.ok").arg(m_backend->appVersion())
		                                                         : T(m_backend, "update.applied.failed").arg(r.value(QStringLiteral("text")).toString(code));
	}
#endif
	// Claude-Anbindung: lokaler Kanal für rj-mcp; freigegebene Aktionen führt das Hauptfenster aus.
	if (!qEnvironmentVariableIsSet("RJ_NO_MCP"))
	{
		auto* mcp = new McpServer(m_backend, this);
		connect(mcp, &McpServer::actionApproved, this, [this](const QString& action, const QString& id) {
			selectId(id);
			const QJsonObject subject{ { QStringLiteral("id"), id } };
			if (action == QLatin1String("verbinden"))
				connectSelected();
			else if (action == QLatin1String("inventar_abfragen"))
			{
				openOverview(subject);
				edit({ { QStringLiteral("op"), QStringLiteral("ov_query") }, { QStringLiteral("id"), id } });
			}
			else
				for (const auto& h : m_modules.mcpAction)
					if (h(action, id))
						break;
			statusBar()->showMessage(T(m_backend, "mcp.action.done"), 8000);
		});
	}
}

MainWindow::~MainWindow() = default;

namespace
{
/// Setzt beim Öffnen jedes Menüs (auch der Erweiterungen) sprechende Symbole, zugeordnet über den Text des Eintrags.
class MenuIconizer : public QObject
{
public:
	MenuIconizer(QObject* parent, QHash<QString, QString> byText)
	  : QObject(parent)
	  , m_byText(std::move(byText))
	{
	}
	static QString norm(QString t)
	{
		t = t.section(QLatin1Char('\t'), 0, 0).remove(QLatin1Char('&')).trimmed();
		if (t.endsWith(QStringLiteral(" …")))
			t.chop(2);
		return t.trimmed();
	}
	void apply(QMenu* m) const
	{
		for (QAction* a : m->actions())
		{
			if (a->isSeparator() || !a->icon().isNull())
				continue;
			QString fa = m_byText.value(norm(a->text()));
			if (fa.isEmpty() && a->menu())
				fa = m_byText.value(norm(a->menu()->title()));
			if (fa.isEmpty() && !m_undoPrefix.isEmpty() && norm(a->text()).startsWith(m_undoPrefix))
				fa = QStringLiteral("rotate-left");
			if (!fa.isEmpty())
				a->setIcon(faIcon(fa));
		}
	}
	bool eventFilter(QObject* o, QEvent* e) override
	{
		if (e->type() == QEvent::Show)
			if (auto* m = qobject_cast<QMenu*>(o))
				apply(m);
		return false;
	}
	QString m_undoPrefix;

private:
	QHash<QString, QString> m_byText;
};
} // namespace

void MainWindow::applyMenuIcons()
{
	// Sprechende Symbole (Font Awesome) je Menüeintrag; zugeordnet über den Text des Eintrags. Gilt für alle Menüs
	// (Hauptmenü, Kontextmenüs, Menüs der Erweiterungen), gesetzt beim Öffnen.
	static const char* const map[][2] = {
		{ "menu.file", "r-file-lines" }, { "menu.edit", "pen-to-square" }, { "menu.extras", "gear" }, { "menu.view", "r-window-restore" },
		{ "menu.help", "circle-question" },
		{ "menu.file.new", "file-circle-plus" }, { "menu.file.open", "r-folder-open" }, { "menu.file.import", "file-import" },
		{ "menu.file.export", "file-export" }, { "menu.file.save", "r-floppy-disk" }, { "menu.file.lock", "lock" },
		{ "menu.file.keyring.forget", "key" }, { "menu.file.quit", "right-from-bracket" },
		{ "menu.edit.undo.none", "rotate-left" }, { "menu.edit.term.copy", "r-copy" }, { "menu.edit.term.paste", "r-paste" },
		{ "menu.edit.properties", "pen-to-square" }, { "menu.edit.add", "plus" }, { "ctx.rename", "i-cursor" },
		{ "ctx.duplicate", "r-clone" }, { "ctx.move", "arrow-right-arrow-left" }, { "ctx.delete", "r-trash-can" },
		{ "ctx.favorite.on", "r-star" }, { "ctx.favorite.off", "star" }, { "menu.extras.settings", "gear" }, { "menu.extras.idle", "hourglass-half" },
		{ "menu.extras.clipclear", "eraser" }, { "menu.extras.shared", "user-group" }, { "menu.extras.credmerge", "object-group" },
		{ "menu.extras.mcplog", "robot" }, { "menu.extras.orphans", "link-slash" }, { "menu.extras.terminal", "terminal" },
		{ "menu.extras.appearance", "palette" }, { "menu.extras.keycapture", "keyboard" }, { "menu.extras.shortcuts", "keyboard" },
		{ "menu.view.single", "r-square" }, { "menu.view.one_by_two", "table-columns" }, { "menu.view.two_by_two", "table-cells-large" },
		{ "menu.view.free", "up-down-left-right" }, { "menu.view.term.bigger", "magnifying-glass-plus" },
		{ "menu.view.term.smaller", "magnifying-glass-minus" }, { "menu.view.term.default", "text-height" },
		{ "menu.view.detach", "up-right-from-square" }, { "menu.view.tree", "thumbtack" }, { "menu.view.reset_window", "rotate" },
		{ "menu.help.update", "arrows-rotate" }, { "menu.help.help", "circle-question" }, { "menu.help.issues", "bug" },
		{ "menu.help.about", "circle-info" }, { "menu.help.news", "r-lightbulb" }, { "menu.help.firststeps", "circle-info" },
		// Kontextmenü des Baums
		{ "ctx.connect", "plug" }, { "ctx.tile", "table-columns" }, { "ctx.copy.user", "user" }, { "ctx.copy.password", "key" },
		{ "ctx.add", "plus" }, { "ctx.properties", "pen-to-square" }, { "ctx.tasks", "list-check" }, { "ctx.mass", "magnifying-glass-chart" },
		{ "ctx.certcheck", "shield-halved" }, { "ctx.credcheck", "user-check" }, { "ctx.credmerge", "object-group" },
		{ "ctx.merge", "arrows-to-circle" }, { "ctx.merge.manual", "object-ungroup" }, { "ctx.up", "arrow-up" }, { "ctx.down", "chevron-down" },
		{ "ctx.all.connect", "plug" }, { "ctx.all.reconnect", "arrows-rotate" }, { "ctx.all.disconnect", "link-slash" },
		{ "ctx.tree.start.save", "r-floppy-disk" }, { "ctx.tree.start.reset", "rotate-left" }, { "login.edit", "pen-to-square" },
		{ "login.delete", "r-trash-can" }, { "ctx.guest.console", "display" }, { "ctx.guest.start", "play" },
		{ "ctx.guest.reboot", "rotate" }, { "ctx.guest.refresh", "arrows-rotate" }, { "fav.connect", "plug" }, { "fav.show", "sitemap" },
	};
	QHash<QString, QString> byText;
	for (const auto& e : map)
		byText.insert(MenuIconizer::norm(T(m_backend, e[0])), QString::fromLatin1(e[1]));
	auto* iconizer = new MenuIconizer(this, byText);
	iconizer->m_undoPrefix = MenuIconizer::norm(T(m_backend, "menu.edit.undo").section(QStringLiteral("%1"), 0, 0));
	qApp->installEventFilter(iconizer);
	std::function<void(QMenu*)> walk = [&](QMenu* m) {
		iconizer->apply(m);
		for (QAction* a : m->actions())
			if (a->menu())
				walk(a->menu());
	};
	for (QAction* a : m_menuBar->actions())
		if (a->menu())
			walk(a->menu());
}

void MainWindow::buildMenus()
{
	QMenu* file = m_menuBar->addMenu(T(m_backend, "menu.file"));
	file->addAction(T(m_backend, "menu.file.new"), this, [this] { m_overlay->openNew(); });
	file->addAction(T(m_backend, "menu.file.open"), QKeySequence::Open, this, [this] { showUnlock(false); });
	file->addAction(T(m_backend, "menu.file.import"), this, [this] {
		const QString path = QFileDialog::getOpenFileName(this, T(m_backend, "menu.file.import"), QString(),
		                                                  QStringLiteral("Exportierte Verbindungslisten und remotejoe-Exporte (*.rdm *.json *.csv *.zip *.rtsz *.rtsx);;Alle Dateien (*)"));
		if (path.isEmpty())
			return;
		QString zipPassword;
		QJsonObject r = parseObject(m_backend->importPreview(path, zipPassword));
		// Verschlüsseltes ZIP (remotejoe-Export): Kennwort abfragen und erneut versuchen.
		while (r.value(QStringLiteral("error")).toString().startsWith(QLatin1String("PASSWORD_")))
		{
			bool ok = false;
			const QString text = T(m_backend, r.value(QStringLiteral("error")).toString() == QLatin1String("PASSWORD_WRONG") ? "import.zip.wrong" : "import.zip.needed");
			zipPassword = QInputDialog::getText(this, T(m_backend, "import.zip.title"), text, QLineEdit::Password, QString(), &ok);
			if (!ok)
				return;
			r = parseObject(m_backend->importPreview(path, zipPassword));
		}
		zipPassword.fill(QLatin1Char('0'));
		if (r.contains(QStringLiteral("error")))
		{
			m_bar->inform(T(m_backend, "import.failed").arg(r.value(QStringLiteral("error")).toString()));
			return;
		}
		openImport(path, r.value(QStringLiteral("report")).toObject(), r.value(QStringLiteral("summary")).toString());
	});
	file->addAction(T(m_backend, "menu.file.export"), this, [this] { openExport(); });
	file->addAction(T(m_backend, "menu.file.save"), QKeySequence::Save, this, [this] {
		// Liegt ein geänderter Eigenschaften-Tab vorn, speichert er seine Felder mit.
		if (auto* ed = qobject_cast<PropertiesTab*>(m_tabs->currentWidget()); ed && ed->isDirty())
			ed->saveNow();
		else
			saveAll();
	});
	file->addAction(T(m_backend, "menu.file.lock"), QKeySequence(QStringLiteral("Ctrl+L")), this, [this] { lockNow(false); });
	file->addSeparator();
	file->addAction(T(m_backend, "menu.file.keyring.forget"), this, [this] { forgetKeyring(); });
	file->addSeparator();
	file->addAction(T(m_backend, "menu.file.quit"), QKeySequence::Quit, this, &QWidget::close);
	QMenu* editMenu = m_menuBar->addMenu(T(m_backend, "menu.edit"));
	m_undoAction = editMenu->addAction(T(m_backend, "menu.edit.undo.none") + QStringLiteral("\tCtrl+Z"), this, [this] { undo(); });
	connect(editMenu, &QMenu::aboutToShow, this, [this] {
		const QString label = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("undo_label") } })).value(QStringLiteral("label")).toString();
		m_undoAction->setText((label.isEmpty() ? T(m_backend, "menu.edit.undo.none") : T(m_backend, "menu.edit.undo").arg(label)) + QStringLiteral("\tCtrl+Z"));
		m_undoAction->setEnabled(!label.isEmpty());
	});
	editMenu->addSeparator();
	editMenu->addAction(T(m_backend, "menu.edit.term.copy"), this, [] { if (auto* t = TerminalView::active()) t->copySelection(); });
	editMenu->addAction(T(m_backend, "menu.edit.term.paste"), this, [] { if (auto* t = TerminalView::active()) t->pasteClipboard(); });
	editMenu->addAction(T(m_backend, "menu.edit.properties") + QStringLiteral("\tF4"), this, [this] {
		if (!currentId().isEmpty())
			openProperties(currentId());
	});
	// „Hinzufügen“ gibt es nur im Kontextmenü des Baums (Strg+N und Einfg wirken weiter im Baum).
	editMenu->addAction(T(m_backend, "ctx.rename") + QStringLiteral("\tF2"), this, [this] { renameSelected(); });
	editMenu->addAction(T(m_backend, "ctx.duplicate") + QStringLiteral("\tCtrl+D"), this, [this] { duplicateSelected(); });
	editMenu->addAction(T(m_backend, "ctx.move") + QStringLiteral("\tCtrl+Shift+M"), this, [this] { moveSelected(); });
	editMenu->addAction(T(m_backend, "ctx.delete") + QStringLiteral("\tEntf"), this, [this] { deleteSelected(); });
	editMenu->addAction(T(m_backend, "ctx.favorite.on") + QStringLiteral("\tCtrl+B"), this, [this] { toggleFavorite(); });
	editMenu->addSeparator();
	QMenu* extras = m_menuBar->addMenu(T(m_backend, "menu.extras"));
	extras->addAction(T(m_backend, "menu.extras.settings"), this, [this] { openSettings(QString()); });
	extras->addSeparator();
	extras->addAction(T(m_backend, "menu.extras.idle"), this, [this] { openSettings(QStringLiteral("lock")); });
	extras->addAction(T(m_backend, "menu.extras.clipclear"), this, [this] { openSettings(QStringLiteral("lock")); });
	extras->addAction(T(m_backend, "menu.extras.shared"), this, [this] { openSharedCheck(); });
	extras->addAction(T(m_backend, "menu.extras.credmerge"), this, [this] { openCredMerge({}); });
	{
		// Einträge der Erweiterungen: neutrales Puzzleteil (der Kern kennt ihre Namen nicht).
		const QList<QAction*> before = extras->actions();
		for (const auto& h : m_modules.extrasMenu)
			h(extras);
		for (QAction* a : extras->actions())
			if (!before.contains(a) && !a->isSeparator() && a->icon().isNull())
				a->setIcon(faIcon(QStringLiteral("puzzle-piece")));
	}
	extras->addAction(T(m_backend, "menu.extras.mcplog"), this, [this] {
		if (auto* old = findChild<McpLogTab*>())
		{
			m_tabs->setCurrentWidget(old);
			return;
		}
		addToolTab(new McpLogTab(m_backend, m_tabs), T(m_backend, "mcplog.tab"));
	});
	extras->addAction(T(m_backend, "menu.extras.orphans"), this, [this] {
		if (auto* old = findChild<OrphanTab*>())
		{
			old->load();
			m_tabs->setCurrentWidget(old);
			return;
		}
		addToolTab(new OrphanTab(m_backend, m_tabs), T(m_backend, "orphans.tab"));
	});
	extras->addAction(T(m_backend, "menu.extras.terminal"), this, [this] { openTerminalSettings(); });
	extras->addAction(T(m_backend, "menu.extras.appearance"), this, [this] { openAppearance(); });
	extras->addSeparator();
	{
		QAction* kc = extras->addAction(T(m_backend, "menu.extras.keycapture"));
		kc->setCheckable(true);
		kc->setChecked(FrameView::s_keyCapture);
		connect(kc, &QAction::toggled, this, [this](bool on) {
			FrameView::s_keyCapture = on;
			if (!on)
				KeyCapture::release();
			edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("keyboard_capture"), on } });
			statusBar()->showMessage(T(m_backend, on ? "keys.on" : "keys.off"), 6000);
			for (auto* st : findChildren<SettingsTab*>())
				st->reload();
		});
		connect(extras, &QMenu::aboutToShow, kc, [kc] { kc->setChecked(FrameView::s_keyCapture); });
	}
	{
		// Tastenkürzel (Strg+Umschalt+W, Strg+F, Tab-Wechsel): eigener Eintrag zur Seite Tastatur der Einstellungen.
		extras->addAction(T(m_backend, "menu.extras.shortcuts"), this, [this] { openSettings(QStringLiteral("keys")); });
	}
	QMenu* help = nullptr;
	QMenu* view = m_menuBar->addMenu(T(m_backend, "menu.view"));
	const char* modes[] = { "single", "one_by_two", "two_by_two", "free" };
	int n = 1;
	for (const char* m : modes)
	{
		const QString mode = QString::fromLatin1(m);
		view->addAction(m_backend->text(QStringLiteral("menu.view.") + mode), QKeySequence(QStringLiteral("Alt+%1").arg(n++)), this, [this, mode] {
			if (m_backend->setArrangement(currentTab(), mode))
				rebuildTabs();
		});
	}
	view->addSeparator();
	view->addAction(T(m_backend, "menu.view.term.bigger"), this, [this] { TerminalView::applyToAll(parseObject(m_backend->terminalFontStep(1))); });
	view->addAction(T(m_backend, "menu.view.term.smaller"), this, [this] { TerminalView::applyToAll(parseObject(m_backend->terminalFontStep(-1))); });
	view->addAction(T(m_backend, "menu.view.term.default"), this, [this] { TerminalView::applyToAll(parseObject(m_backend->terminalFontStep(0))); });
	view->addSeparator();
	view->addAction(T(m_backend, "menu.view.detach"), QKeySequence(QStringLiteral("Ctrl+Shift+D")), this, [this] {
		if (m_backend->detachTab(currentTab()))
			rebuildTabs();
	});
	view->addSeparator();
	view->addAction(T(m_backend, "menu.view.tree"), QKeySequence(Qt::Key_F9), this, [this] { setTreeVisible(m_treeHidden); });
	view->addAction(T(m_backend, "menu.view.reset_window"), this, [this] { resetWindow(); });
	help = m_menuBar->addMenu(T(m_backend, "menu.help"));
	help->addAction(T(m_backend, "menu.help.news"), this, [this] { openNews(); });
	help->addAction(T(m_backend, "menu.help.firststeps"), this, [this] { showFirstHints(true); });
	help->addAction(T(m_backend, "menu.help.update"), this, [this] { openUpdateTab(); });
	help->addAction(T(m_backend, "menu.help.help"), QKeySequence::HelpContents, this, [] { QDesktopServices::openUrl(QUrl(QString::fromLatin1(kHelpUrl))); });
	help->addAction(T(m_backend, "menu.help.issues"), this, [this] {
		if (auto* old = findChild<ReportTab*>())
		{
			m_tabs->setCurrentWidget(old);
			return;
		}
		addToolTab(new ReportTab(m_backend, m_tabs), T(m_backend, "report.tab"));
	});
	help->addSeparator();
	// Das einzige Popup des Programms: „Über remotejoe“.
	help->addAction(T(m_backend, "menu.help.about"), this, [this] {
		auto* d = new AboutDialog(m_backend, m_backend->appVersion(), this);
		d->open();
	});
}

QWidget* MainWindow::aboutForScreenshot()
{
	auto* d = new AboutDialog(m_backend, m_backend->appVersion(), this);
	d->show();
	return d;
}

void MainWindow::updateForScreenshot(const QString& base)
{
	QJsonObject o;
	o.insert(QStringLiteral("op"), QStringLiteral("update_demo_source"));
	o.insert(QStringLiteral("base_url"), base);
	m_backend->edit(QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Compact)));
	openUpdateTab();
	if (auto* u = findChild<UpdateTab*>())
		u->check();
}

void MainWindow::openUpdateTab()
{
	// Programm-Updates stehen unter Einstellungen → Updates → Programm-Updates.
	openSettings(QStringLiteral("updates_program"));
	if (auto* u = findChild<UpdateTab*>())
		u->refresh();
}

/// Stylesheet der Kartenbausteine (cards.h) aus den Farben des Schemas; Plaketten hell und dunkel lesbar.
static QString cardStyle(const QColor& base, const QColor& window, const QColor& alternate, const QColor& muted, const QColor& text, const QColor& accent)
{
	const bool dark = base.lightness() < 128;
	auto mix = [](const QColor& a, const QColor& b, int pctA) {
		return QColor((a.red() * pctA + b.red() * (100 - pctA)) / 100, (a.green() * pctA + b.green() * (100 - pctA)) / 100, (a.blue() * pctA + b.blue() * (100 - pctA)) / 100);
	};
	const QColor border = mix(text, base, 18);
	const QColor tileBg = dark ? mix(text, base, 7) : mix(window, base, 60);
	const QColor okText = dark ? QColor("#7fd69a") : QColor("#23793a"), okBg = mix(QColor("#2e9e44"), base, dark ? 30 : 16);
	const QColor warnText = dark ? QColor("#f2c174") : QColor("#9a5b08"), warnBg = mix(QColor("#d98a13"), base, dark ? 30 : 18);
	const QColor accentBg = mix(accent, base, dark ? 30 : 16);
	const QColor rowSel = mix(accent, base, dark ? 22 : 12);
	const QColor fieldBg = dark ? mix(text, base, 9) : base.lighter(104);
	const QColor fieldBorder = mix(text, base, dark ? 32 : 28);
	Q_UNUSED(alternate);
	return QStringLiteral("QFrame#rjCard { background: %1; border: 1px solid %2; border-radius: 10px; }"
	                      "QFrame#rjTile { background: %3; border-radius: 6px; }"
	                      "QFrame#rjTile[rjAlarm=\"true\"] { background: %16; }"
	                      "QFrame#rjTile[rjAlarm=\"true\"] QLabel { color: %15; }"
	                      "QLabel#rjSection, QLabel#rjTileLabel, QLabel#rjMuted { color: %4; }"
	                      "QLabel[rjPill=\"ok\"] { background: %5; color: %6; border-radius: 9px; padding: 1px 9px; }"
	                      "QLabel[rjPill=\"warn\"] { background: %7; color: %8; border-radius: 9px; padding: 1px 9px; }"
	                      "QLabel[rjPill=\"off\"] { background: %3; color: %4; border-radius: 9px; padding: 1px 9px; }"
	                      "QLabel[rjPill=\"accent\"] { background: %9; color: %10; border-radius: 9px; padding: 1px 9px; }"
	                      "QFrame#rjRow { border-radius: 6px; }"
	                      "QFrame#rjApTile { border: 1px solid %2; border-radius: 6px; }"
	                      "QFrame#rjApTile:hover { background: %3; }"
	                      "QLabel[rjDot=\"ok\"] { color: %6; }"
	                      "QLabel[rjDot=\"err\"] { color: %15; }"
	                      "QLabel[rjDot=\"warn\"] { color: %8; }"
	                      "QLabel[rjPill=\"err\"] { background: %16; color: %15; border-radius: 9px; padding: 1px 9px; }"
	                      "QLabel[rjWarnText=\"true\"] { color: %8; }"
	                      "QLabel[rjErrText=\"true\"] { color: %15; font-weight: bold; }"
	                      "QLabel[rjPill=\"yellow\"] { background: %17; color: %18; border-radius: 9px; padding: 1px 9px; }"
	                      "QFrame#rjRow:hover { background: %3; }"
	                      "QFrame#rjRow[selected=\"true\"] { background: %11; }"
	                      // Eingabefelder in Karten: eigener Rand und leicht abgesetzter Hintergrund (bei dunklen
	                      // Schemata hätten Feld und Karte sonst dieselbe Farbe). Klappfelder bleiben beim Stil.
	                      "QFrame#rjCard QLineEdit, QFrame#rjCard QPlainTextEdit, QFrame#rjCard QTextEdit, QFrame#rjCard QSpinBox"
	                      " { background: %12; border: 1px solid %13; border-radius: 4px; padding: 2px 4px; }"
	                      "QFrame#rjCard QLineEdit:focus, QFrame#rjCard QPlainTextEdit:focus, QFrame#rjCard QTextEdit:focus, QFrame#rjCard QSpinBox:focus"
	                      " { border-color: %14; }"
	                      "QFrame#rjCard QLineEdit:disabled, QFrame#rjCard QPlainTextEdit:disabled, QFrame#rjCard QSpinBox:disabled"
	                      " { background: %3; color: %4; }"
	                      // Zahlenfelder: Mit eigenem Rand zeichnet Qt die Pfeilknöpfe nicht mehr; darum eigene Knöpfe.
	                      "QFrame#rjCard QSpinBox { padding-right: 18px; }"
	                      "QFrame#rjCard QSpinBox::up-button { subcontrol-origin: border; subcontrol-position: top right; width: 18px; border: none; }"
	                      "QFrame#rjCard QSpinBox::down-button { subcontrol-origin: border; subcontrol-position: bottom right; width: 18px; border: none; }"
	                      "QFrame#rjCard QSpinBox::up-arrow { image: url(:/ui/spin-up.png); width: 9px; height: 5px; }"
	                      "QFrame#rjCard QSpinBox::down-arrow { image: url(:/ui/spin-down.png); width: 9px; height: 5px; }"
	                      "QFrame#rjCard QSpinBox::up-button:hover, QFrame#rjCard QSpinBox::down-button:hover { background: %3; }"
	                      "QFrame#rjCard *[rjDirty=\"true\"]:enabled { border: 2px solid %14; }")
	  .arg(base.name(), border.name(), tileBg.name(), muted.name(), okBg.name(), okText.name(), warnBg.name(), warnText.name(), accentBg.name())
	  .arg(dark ? accent.lighter(140).name() : accent.darker(120).name(), rowSel.name(), fieldBg.name(), fieldBorder.name(), accent.name())
	  .arg(dark ? QStringLiteral("#f28b82") : QStringLiteral("#b3261e"), mix(QColor("#d93025"), base, dark ? 30 : 16).name())
	  .arg(mix(QColor("#f2c200"), base, dark ? 30 : 28).name(), dark ? QStringLiteral("#ffe08a") : QStringLiteral("#6b5200"));
}

void MainWindow::applyTheme(bool dark)
{
	qApp->setStyle(QStyleFactory::create(QStringLiteral("Fusion")));
	QJsonObject p = parseObject(m_backend->paletteJson());
	if (p.value(QStringLiteral("mode")).toString() != QLatin1String("system") && qEnvironmentVariableIsSet("REMOTEJOE_THEME") && dark != (QColor(p.value(QStringLiteral("base")).toString()).lightness() < 128))
	{
		// Gegenteiliges Schema erzwingen (für Bildschirmfotos).
		qputenv("REMOTEJOE_THEME", dark ? "dark" : "light");
		p = parseObject(m_backend->paletteJson());
	}
	// „System“: Farben aus der Stilpalette (hell oder dunkel und Akzentfarbe vom Desktop);
	// sonst ein Standardschema oder eine eigene Palette.
	const bool system = p.value(QStringLiteral("mode")).toString() == QLatin1String("system");
	QPalette pal = system ? qApp->style()->standardPalette() : QPalette();
	auto c = [&](const char* k) -> QColor {
		if (system)
		{
			const QString key = QString::fromLatin1(k);
			if (key == QLatin1String("window"))
				return pal.color(QPalette::Window);
			if (key == QLatin1String("text"))
				return pal.color(QPalette::WindowText);
			if (key == QLatin1String("base"))
				return pal.color(QPalette::Base);
			if (key == QLatin1String("alternate"))
				return pal.color(QPalette::AlternateBase);
			if (key == QLatin1String("muted"))
				return pal.color(QPalette::PlaceholderText);
			if (key == QLatin1String("button"))
				return pal.color(QPalette::Button);
			if (key == QLatin1String("highlight"))
				return pal.color(QPalette::Highlight);
			if (key == QLatin1String("highlight_text"))
				return pal.color(QPalette::HighlightedText);
			if (key == QLatin1String("link"))
				return pal.color(QPalette::Link);
		}
		return QColor(p.value(QString::fromLatin1(k)).toString());
	};
	if (!system)
	{
		pal.setColor(QPalette::Window, c("window"));
		pal.setColor(QPalette::WindowText, c("text"));
		pal.setColor(QPalette::Base, c("base"));
		pal.setColor(QPalette::AlternateBase, c("alternate"));
		pal.setColor(QPalette::Text, c("text"));
		pal.setColor(QPalette::PlaceholderText, c("muted"));
		pal.setColor(QPalette::Button, c("button"));
		pal.setColor(QPalette::ButtonText, c("text"));
		pal.setColor(QPalette::Highlight, c("highlight"));
		pal.setColor(QPalette::HighlightedText, c("highlight_text"));
		pal.setColor(QPalette::Link, c("link"));
		pal.setColor(QPalette::ToolTipBase, c("base"));
		pal.setColor(QPalette::ToolTipText, c("text"));
		// Gesperrte Bedienelemente sichtbar abheben: gedämpfte Schrift, Feldhintergrund wie das Fenster.
		for (const auto role : { QPalette::WindowText, QPalette::Text, QPalette::ButtonText })
			pal.setColor(QPalette::Disabled, role, c("muted"));
		pal.setColor(QPalette::Disabled, QPalette::Base, c("window"));
	}
	qApp->setPalette(pal);
	// Trennlinie der Tabs: Mischung aus Text- und Fensterfarbe, in beiden Schemata gut sichtbar.
	const QColor textColor = c("text"), windowColor = c("window");
	const QColor tabLine((textColor.red() * 55 + windowColor.red() * 45) / 100, (textColor.green() * 55 + windowColor.green() * 45) / 100,
	                     (textColor.blue() * 55 + windowColor.blue() * 45) / 100);
	m_starColor = c("warning").isValid() ? c("warning") : QColor("#d4a017");
	// Symbolfarben des Themes; danach Baum und Tabs neu zeichnen.
	{
		QJsonObject icons = p.value(QStringLiteral("icons")).toObject();
		icons.insert(QStringLiteral("other"), c("muted").name());
		setIconColors(icons, c("base"), c("highlight_text"), c("highlight"));
	}
	// Baumgröße: Zeilenhöhe, Symbole und Schrift in drei Stufen.
	const QString treeSize = p.value(QStringLiteral("tree_size")).toString(QStringLiteral("mittel"));
	const int rowHeight = treeSize == QLatin1String("klein") ? 20 : treeSize == QLatin1String("gross") ? 30 : 24;
	const int iconSize = treeSize == QLatin1String("klein") ? 16 : treeSize == QLatin1String("gross") ? 24 : 20;
	const qreal fontDelta = treeSize == QLatin1String("klein") ? -1.0 : treeSize == QLatin1String("gross") ? 2.0 : 0.0;
	if (m_tree)
	{
		m_tree->setIconSize(QSize(iconSize, iconSize));
		QFont f = qApp->font();
		if (f.pointSizeF() > 0)
			f.setPointSizeF(f.pointSizeF() + fontDelta);
		m_tree->setFont(f);
	}
	// Schließen-Kreuz der Tabs (Font Awesome) in Neben- und Textfarbe; Stylesheets brauchen dafür eine Datei.
	// Feine Linien der Tabs: Trennstrich, Kartenrand und „Schatten“ (Text- zu Fensterfarbe gemischt).
	auto mix = [&](qreal t) {
		const QColor a = c("text"), b = c("window");
		return QColor::fromRgbF(a.redF() * t + b.redF() * (1 - t), a.greenF() * t + b.greenF() * (1 - t), a.blueF() * t + b.blueF() * (1 - t)).name();
	};
	auto withCloseIcons = [&](QString css) {
		css.replace(QStringLiteral("@SEP@"), mix(0.22));
		css.replace(QStringLiteral("@CARD@"), mix(0.16));
		css.replace(QStringLiteral("@SHADOW@"), mix(0.30));
		css.replace(QStringLiteral("@CLOSEHOVER@"), faFile(QStringLiteral("xmark"), c("text")));
		css.replace(QStringLiteral("@CLOSE@"), faFile(QStringLiteral("xmark"), c("muted")));
		return css;
	};
	qApp->setStyleSheet(withCloseIcons(QStringLiteral(
	  "QTreeView::item { min-height: %9px; padding: 0 2px; }"
	  // Tableiste zeichnet RjTabBar selbst; hier nur der Inhalt ohne Rahmen und das Schließen-Kreuz.
	  "QTabWidget::pane { border: none; top: 0; }"
	  "QTabBar::close-button { subcontrol-position: right; margin-right: 8px; image: url(\"@CLOSE@\"); "
	  "width: 12px; height: 12px; padding: 2px; border-radius: 4px; }"
	  "QTabBar::close-button:hover { background: %4; image: url(\"@CLOSEHOVER@\"); }"
	  "QTabBar::scroller { width: 24px; }"
	  "QTabBar::tab:disabled { color: %5; border-color: %3; }"
	  // Platzhalter %6–%8 bleiben belegt (sonst verschiebt QString::arg die Farben).
	  "QLabel#rjPh6 { color: %6; } QLabel#rjPh7 { color: %7; } QLabel#rjPh8 { color: %8; }"
	  "QLabel#paneHeader { font-weight: bold; padding: 1px 4px; }"
	  "QLabel#paneNotice { color: %1; padding: 4px; }"
	  "QPushButton { min-height: 30px; padding: 2px 12px; }"
	  "QPushButton#paneBtn { min-height: 16px; padding: 0 8px; }"
	  "QPushButton[rjCompact=\"true\"] { min-height: 20px; padding: 1px 9px; }"
	  "QStatusBar { padding: 0; margin: 0; } QStatusBar::item { border: none; }"
	  "QStatusBar QLabel { padding: 0; margin: 0; }"
	  "QStatusBar QPushButton { min-height: 12px; padding: 0 6px; }"
	  "QToolButton#statusTreeButton { padding: 0 6px; margin: 0; min-height: 0; }"
	  "QLabel#statusNotice { color: %1; }"
	  "QLabel#statusNotice[severe=\"true\"] { color: #e53935; font-weight: bold; }"
	  "QLabel#fieldError, QLabel#dirtyLabel { color: %1; font-weight: bold; }"
	  "QSplitter::handle:hover { background: %2; }"
	  "QFrame#actionBar { border: 2px solid %2; border-radius: 4px; }"
	  "QFrame#securityBar { border: 2px solid %2; border-radius: 4px; }"
	  "QFrame#securityBar[changed=\"true\"] { border: 3px solid %1; }"
	  "QFrame#securityBar[changed=\"true\"] QLabel#securityTitle { color: %1; font-weight: bold; }"
	  "QLabel#securityTitle { font-weight: bold; }"
	  "QLabel#heading { font-weight: bold; padding: 4px 0; }"
	  "QFrame#unlockBox { border: 2px solid %2; border-radius: 6px; padding: 12px; }"
	  // Optionsknöpfe und Kästchen markiert der Eigenschaften-Tab über die Palette (Textfarbe), nicht hier.
	  "*[rjDirty=\"true\"]:enabled { border: 2px solid %2; }")
	                      .arg(c("warning").name(), c("highlight").name())
	                      .arg(tabLine.name(), c("button").name(), c("muted").name(), c("base").name(), c("text").name(), c("alternate").name())
	                      .arg(rowHeight)
	                  // Feste Farbschemata: Der Stil leitet den Rand der Optionsknöpfe und Kästchen aus der Fensterfarbe ab;
	                  // bei dunklen Schemata wäre er unsichtbar. Darum eigener Rand und Füllung in der Akzentfarbe.
	                  + (system ? QString()
	                            : QStringLiteral("QRadioButton::indicator, QCheckBox::indicator { width: 13px; height: 13px; border: 1px solid %1; background: %2; }"
	                                             "QRadioButton::indicator { border-radius: 7px; }"
	                                             "QCheckBox::indicator { border-radius: 2px; }"
	                                             "QRadioButton::indicator:checked { background: qradialgradient(cx:0.5, cy:0.5, radius:0.5, fx:0.5, fy:0.5, stop:0 %3, stop:0.5 %3, stop:0.6 %2, stop:1 %2); border-color: %3; }"
	                                             "QCheckBox::indicator:checked { background: %3; border-color: %3; }"
	                                             "QRadioButton::indicator:disabled, QCheckBox::indicator:disabled { border-color: %4; background: %5; }")
	                                  .arg(c("muted").name(), c("base").name(), c("highlight").name(), c("alternate").name(), c("window").name()))
	                  // Kartenstil (Übersicht, Einstellungen): Karten, Kacheln, Abschnitte, Plaketten, Zeilen der Bereichsliste.
	                  + cardStyle(c("base"), c("window"), c("alternate"), c("muted"), c("text"), c("highlight"))));
	if (m_model)
		reloadTree();
	if (m_tabs)
		rebuildTabs();
}

void MainWindow::reloadTree()
{
	// Offene Übersicht: Inventar-Schalter nachziehen (z. B. nach dem Speichern der Einstellungen).
	if (auto* o = findChild<OverviewTab*>())
		QTimer::singleShot(0, o, [o] { o->refreshInventorySwitch(); });
	// Offene Prüfung „Geteilte Zugangsdaten“: neu laden (z. B. nach Bearbeiten → Rückgängig).
	if (auto* sc = findChild<SharedCredTab*>())
		QTimer::singleShot(0, sc, [sc] { sc->load(); });
	// Offene Vorschau „Zusammenfassen“: Gruppen neu laden (z. B. nach Bearbeiten → Rückgängig).
	if (auto* m = findChild<MergeTab*>())
		QTimer::singleShot(0, m, [m] { m->refresh(); });
	// Aufgeklappte Ordner, Auswahl und Bildlauf merken, damit eine Änderung den Baum nicht zuklappt.
	// Während einer Suche klappt alles auf; dann bleibt der zuletzt bekannte Zustand ohne Suche erhalten. Gemerkt wird
	// nur, solange der Baum noch nicht durch die Suche aufgeklappt ist (auch wenn der Suchtext schon getippt ist).
	const bool searching = !m_search->text().isEmpty();
	if (m_treeLoaded && !m_searchApplied)
	{
		m_expandedIds.clear();
		std::function<void(const QModelIndex&)> collect = [&](const QModelIndex& parent) {
			for (int r = 0; r < m_model->rowCount(parent); ++r)
			{
				const QModelIndex i = m_model->index(r, 0, parent);
				if (m_tree->isExpanded(i))
				{
					const QString id = i.data(RoleId).toString();
					if (!id.isEmpty())
						m_expandedIds.insert(id);
				}
				collect(i);
			}
		};
		collect(QModelIndex());
	}
	const QString current = m_tree->currentIndex().isValid() ? m_tree->currentIndex().data(RoleId).toString() : QString();
	const int scroll = m_tree->verticalScrollBar()->value();

	// Immer den ganzen Baum aufbauen; eine Suche blendet danach nur Zeilen aus (applySearch).
	const QString json = m_backend->treeJson(QString());
	m_tree->setUpdatesEnabled(false);
	m_model->clear();
	addNodes(m_model->invisibleRootItem(), QJsonDocument::fromJson(json.toUtf8()).array());
	addFavoritesFolder();
	m_searchApplied = false;
	if (searching)
	{
		// Aufklappen und Ausblenden übernimmt applySearch.
	}
	else if (m_treeLoaded)
	{
		std::function<void(const QModelIndex&)> restore = [&](const QModelIndex& parent) {
			for (int r = 0; r < m_model->rowCount(parent); ++r)
			{
				const QModelIndex i = m_model->index(r, 0, parent);
				// Hypervisoren bleiben zu: ihre Gäste lädt erst der Pfeil (sonst eine Abfrage je Neuaufbau).
				const QModelIndex first = m_model->index(0, 0, i);
				if (m_expandedIds.contains(i.data(RoleId).toString()) && !(first.isValid() && first.data(RolePlaceholder).toBool()))
					m_tree->expand(i);
				restore(i);
			}
		};
		m_autoExpand = true;
		restore(QModelIndex());
		m_autoExpand = false;
	}
	else
	{
		m_autoExpand = true;
		// Gespeicherter Startzustand (Kontextmenü „Aufklappzustand als Start speichern“), sonst die Vorgabe.
		const QJsonValue saved = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("tree_start") } })).value(QStringLiteral("ids"));
		if (saved.isArray())
		{
			QSet<QString> ids;
			for (const QJsonValue& v : saved.toArray())
				ids.insert(v.toString());
			std::function<void(const QModelIndex&)> open = [&](const QModelIndex& parent) {
				for (int r = 0; r < m_model->rowCount(parent); ++r)
				{
					const QModelIndex i = m_model->index(r, 0, parent);
					const QModelIndex first = m_model->index(0, 0, i);
					if (ids.contains(i.data(RoleId).toString()) && !(first.isValid() && first.data(RolePlaceholder).toBool()))
						m_tree->expand(i);
					open(i);
				}
			};
			open(QModelIndex());
		}
		else
		{
			m_tree->expandToDepth(0);
			// Erster Start: den ersten Ordner mit Zielen aufklappen.
			if (m_model->rowCount() > 0)
				m_tree->expand(m_model->index(0, 0, m_model->index(0, 0)));
		}
		collapseUnloadedHypervisors();
		m_autoExpand = false;
	}
	m_treeLoaded = true;
	addMacroShelf();
	// Offene Ordnerübersicht mit dem neuen Baum auffrischen (z. B. nach dem Anlegen eines Ziels).
	if (auto* fo = findChild<FolderOverviewTab*>(); fo && !fo->folderId().isEmpty())
	{
		std::function<QModelIndex(const QModelIndex&)> find = [&](const QModelIndex& parent) -> QModelIndex {
			for (int r = 0; r < m_model->rowCount(parent); ++r)
			{
				const QModelIndex i = m_model->index(r, 0, parent);
				if (i.data(RoleId).toString() == fo->folderId())
					return i;
				if (const QModelIndex hit = find(i); hit.isValid())
					return hit;
			}
			return {};
		};
		if (const QModelIndex i = find(QModelIndex()); i.isValid())
			openFolderOverview(i, false);
	}
	if (!current.isEmpty())
	{
		std::function<QModelIndex(const QModelIndex&)> find = [&](const QModelIndex& parent) -> QModelIndex {
			for (int r = 0; r < m_model->rowCount(parent); ++r)
			{
				const QModelIndex i = m_model->index(r, 0, parent);
				if (i.data(RoleId).toString() == current && !i.data(RoleFavLink).toBool())
					return i;
				if (const QModelIndex f = find(i); f.isValid())
					return f;
			}
			return QModelIndex();
		};
		if (const QModelIndex i = find(QModelIndex()); i.isValid())
			m_tree->setCurrentIndex(i);
	}
	m_tree->setUpdatesEnabled(true);
	updateSessionMarks();
	if (searching)
	{
		// Der neu aufgebaute Baum ist noch zugeklappt: den gemerkten Zustand ohne Suche nicht überschreiben.
		m_keepExpanded = true;
		applySearch();
		m_keepExpanded = false;
	}
	else
		m_tree->verticalScrollBar()->setValue(scroll);
	updateEmptyHint();
}

void MainWindow::updateSessionMarks()
{
	if (!m_model)
		return;
	// Ziel → 1 (mindestens eine verbundene Sitzung) bzw. 2 (nur offene, nicht verbundene Tabs).
	QHash<QString, int> marks;
	for (auto it = m_panes.cbegin(); it != m_panes.cend(); ++it)
	{
		const QJsonObject info = parseObject(m_backend->sessionInfo(it.key()));
		const QString target = info.value(QStringLiteral("target")).toString();
		if (target.isEmpty())
			continue;
		const int m = info.value(QStringLiteral("state")).toString() == QLatin1String("connected") ? 1 : 2;
		marks[target] = marks.contains(target) ? qMin(marks.value(target), m) : m;
	}
	std::function<void(QStandardItem*)> walk = [&](QStandardItem* parent) {
		for (int r = 0; r < parent->rowCount(); ++r)
		{
			QStandardItem* i = parent->child(r);
			const QString id = i->data(RoleId).toString();
			if (!id.isEmpty() && !i->data(RoleFolder).toBool() && !i->data(RoleGuest).isValid() && !i->data(RoleMacro).toInt())
			{
				const int m = marks.value(id, 0);
				if (i->data(RoleSessionMark).toInt() != m)
				{
					i->setData(m, RoleSessionMark);
					const QString kind = i->data(RoleIconKind).toString();
					i->setIcon(protocolIcon(kind.isEmpty() ? i->data(RoleProtocol).toString() : kind, i->data(RoleLight).toString(), false, m));
				}
			}
			walk(i);
		}
	};
	walk(m_model->invisibleRootItem());
}

void MainWindow::applySearch()
{
	QElapsedTimer t;
	t.start();
	const QString text = m_search->text();
	if (text.isEmpty())
	{
		// Suche verlassen: einmal neu aufbauen, damit Aufklappzustand und Makro-Fach zurückkommen.
		// m_searchApplied bleibt dabei gesetzt, damit reloadTree den aufgeklappten Suchbaum nicht als Zustand merkt.
		if (m_searchApplied)
			reloadTree();
		return;
	}
	if (!m_searchApplied && m_treeLoaded && !m_keepExpanded)
	{
		// Zustand ohne Suche merken (wie in reloadTree).
		m_expandedIds.clear();
		std::function<void(const QModelIndex&)> collect = [&](const QModelIndex& parent) {
			for (int r = 0; r < m_model->rowCount(parent); ++r)
			{
				const QModelIndex i = m_model->index(r, 0, parent);
				if (m_tree->isExpanded(i) && !i.data(RoleId).toString().isEmpty())
					m_expandedIds.insert(i.data(RoleId).toString());
				collect(i);
			}
		};
		collect(QModelIndex());
	}
	// Treffer samt übergeordneter Ordner liefert Rust; hier werden nur Zeilen aus- und eingeblendet.
	QSet<QString> keep;
	std::function<void(const QJsonArray&)> ids = [&](const QJsonArray& nodes) {
		for (const QJsonValue& v : nodes)
		{
			const QJsonObject n = v.toObject();
			keep.insert(n.value(QStringLiteral("id")).toString());
			ids(n.value(QStringLiteral("children")).toArray());
		}
	};
	ids(QJsonDocument::fromJson(m_backend->treeJson(text).toUtf8()).array());
	const qint64 rustMs = t.elapsed();
	m_tree->setUpdatesEnabled(false);
	std::function<void(const QModelIndex&)> apply = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			const QString id = i.data(RoleId).toString();
			// Einträge ohne Kennung (Gäste, Platzhalter) folgen ihrem Hypervisor; das Makro-Fach nie.
			const bool hide = i.data(RoleMacro).toInt() == 1 || i.data(RoleFavFolder).toBool() || (!id.isEmpty() && !keep.contains(id));
			m_tree->setRowHidden(r, parent, hide);
			if (!hide && !i.data(RoleGuest).isValid())
				apply(i);
		}
	};
	apply(QModelIndex());
	m_autoExpand = true;
	m_tree->expandAll();
	collapseUnloadedHypervisors();
	m_autoExpand = false;
	m_tree->setUpdatesEnabled(true);
	m_searchApplied = true;
	if (qEnvironmentVariableIsSet("RJ_TIMING"))
		std::fprintf(stderr, "Suche „%s“: Treffer %lld ms, gesamt %lld ms, %lld Einträge\n", qPrintable(text), static_cast<long long>(rustMs),
		             static_cast<long long>(t.elapsed()), static_cast<long long>(keep.size()));
	updateEmptyHint();
}

void MainWindow::addMacroShelf()
{
	// Das Fach erscheint nur in der Ansicht „Alle Ziele“ und ohne Suchtext.
	if (!m_search->text().isEmpty() || !m_viewBox->currentData().toString().isEmpty())
		return;
	const QJsonArray macros = QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("macro_list") } }).toUtf8()).array();
	auto* shelf = new QStandardItem(protocolIcon(QString(), QString(), true), T(m_backend, "macro.shelf"));
	shelf->setData(1, RoleMacro);
	shelf->setEditable(false);
	shelf->setDragEnabled(false);
	shelf->setDropEnabled(false);
	for (const QJsonValue& v : macros)
	{
		const QJsonObject m = v.toObject();
		auto* it = new QStandardItem(protocolIcon(QStringLiteral("macro"), QString(), false), m.value(QStringLiteral("name")).toString());
		it->setData(2, RoleMacro);
		it->setData(m.value(QStringLiteral("id")).toString(), RoleId);
		it->setEditable(false);
		it->setDragEnabled(false);
		it->setDropEnabled(false);
		it->setToolTip(T(m_backend, "macro.links").arg(m.value(QStringLiteral("links")).toInt()));
		shelf->appendRow(it);
	}
	m_model->invisibleRootItem()->appendRow(shelf);
	if (m_macroShelfOpen)
		m_tree->expand(shelf->index());
}

void MainWindow::updateEmptyHint()
{
	if (!m_emptyHint)
		return;
	m_emptyHint->setGeometry(m_tree->viewport()->rect());
	int rows = m_model->rowCount();
	for (int r = 0; r < m_model->rowCount(); ++r)
		if (m_model->index(r, 0).data(RoleMacro).toInt() == 1)
			--rows;
	// Zustand des Hinweises: gesperrt (keine Datenbank entsperrt) oder leer (entsperrt, keine Ziele).
	const bool open = m_backend->isVaultOpen();
	m_emptyLabel->setText(T(m_backend, open ? "tree.empty" : "tree.locked"));
	m_emptyButton->setText(T(m_backend, open ? "tree.empty.add" : "tree.locked.open"));
	m_emptyHint->setVisible((rows <= 0 || !open) && m_search->text().isEmpty());
	m_emptyHint->raise();
}

int MainWindow::treeRowCount() const
{
	int n = 0;
	std::function<void(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			// Platzhalter und Gäste von Hypervisoren zählen nicht als Index-Einträge.
			if (i.data(RolePlaceholder).toBool() || i.data(RoleGuest).isValid() || i.data(RoleMacro).toInt())
				continue;
			++n;
			walk(i);
		}
	};
	walk(QModelIndex());
	return n;
}

void MainWindow::setFilter(const QString& text)
{
	m_search->setText(text);
	reloadTree();
}

QSize MainWindow::contentSize(bool terminal) const
{
	QSize s = m_tabs->size() - QSize(8, 40);
	if (!terminal)
		return s;
	QFontMetrics fm(QFontDatabase::systemFont(QFontDatabase::FixedFont));
	return QSize(qMax(20, s.width() / qMax(1, fm.horizontalAdvance(QLatin1Char('M')))), qMax(5, s.height() / qMax(1, fm.height())));
}

void MainWindow::connectSelected(const QString& credential, bool remember)
{
	// Gelöste Stecknadel: nach dem Verbinden klappt der Zielbereich zu.
	if (!m_pinned && m_flyOpen)
		QTimer::singleShot(0, this, [this] { closeFlyout(); });
	// Module (z. B. Einträge, die keine Ziele sind): öffnen statt verbinden.
	for (const auto& h : m_modules.activate)
		if (h(m_tree->currentIndex()))
			return;
	// Anmeldeobjekt: Bearbeitung statt Verbindung.
	if (m_tree->currentIndex().data(RoleLogin).toBool() && m_tree->selectionModel()->selectedIndexes().size() <= 1)
	{
		openLogin(m_tree->currentIndex().data(RoleId).toString(), QString());
		return;
	}
	// Mehrere markierte Ziele: alle gestaffelt öffnen (Ordner in der Markierung zählen nicht).
	QStringList ids;
	for (const QModelIndex& i : m_tree->selectionModel()->selectedIndexes())
		if (!i.data(RoleFolder).toBool() && !i.data(RoleGuest).isValid() && !i.data(RolePlaceholder).toBool() && !i.data(RoleMacro).toInt() && !i.data(RoleLogin).toBool())
			ids << i.data(RoleId).toString();
	ids.removeDuplicates();
	if (ids.size() > 1 && credential.isEmpty())
	{
		connectMany({ { QStringLiteral("ids"), QJsonArray::fromStringList(ids) } });
		return;
	}
	connectCurrent(credential, remember);
}

bool MainWindow::confirmManyTargets(int n)
{
	if (n <= kConnectWithoutAsking)
		return true;
	QMessageBox box(QMessageBox::Question, T(m_backend, "connect.many.title"), T(m_backend, "connect.many.text").arg(n), QMessageBox::NoButton, this);
	auto* ok = box.addButton(T(m_backend, "connect.many.ok"), QMessageBox::AcceptRole);
	box.setDefaultButton(box.addButton(T(m_backend, "connect.many.cancel"), QMessageBox::RejectRole));
	box.exec();
	return box.clickedButton() == ok;
}

void MainWindow::connectMany(const QJsonObject& which)
{
	QJsonObject count = which;
	count.insert(QStringLiteral("op"), QStringLiteral("connect_all"));
	count.insert(QStringLiteral("count_only"), true);
	if (!confirmManyTargets(parseObject(edit(count)).value(QStringLiteral("count")).toInt()))
		return;
	const QSize px = contentSize(false);
	QJsonObject cmd = which;
	cmd.insert(QStringLiteral("op"), QStringLiteral("connect_all"));
	cmd.insert(QStringLiteral("mode"), QStringLiteral("connect"));
	cmd.insert(QStringLiteral("width"), px.width());
	cmd.insert(QStringLiteral("height"), px.height());
	const QJsonObject r = parseObject(edit(cmd));
	if (r.contains(QStringLiteral("error")))
		statusBar()->showMessage(r.value(QStringLiteral("error")).toString(), 8000);
	else
		statusBar()->showMessage(T(m_backend, "status.connect_all").arg(r.value(QStringLiteral("count")).toInt()).arg(4), 8000);
	rebuildTabs();
}

void MainWindow::connectCurrent(const QString& credential, bool remember)
{
	const QModelIndex i = m_tree->currentIndex();
	if (!i.isValid() || i.data(RoleFolder).toBool())
		return;
	const QString id = i.data(RoleId).toString();
	const bool term = m_tree->currentIndex().data(Qt::ToolTipRole).isValid();
	Q_UNUSED(term);
	const QSize px = contentSize(false);
	const QString res = m_backend->connectTarget(id, credential, remember, px.width(), px.height());
	handleConnectResult(id, res);
}

void MainWindow::handleConnectResult(const QString& id, const QString& json)
{
	const QJsonObject r = parseObject(json);
	if (r.contains(QStringLiteral("pending")))
	{
		// Tunnel oder Konsole wird im Hintergrund aufgebaut.
		m_jobTargets.insert(qulonglong(r.value(QStringLiteral("pending")).toInteger()), id);
		updateJobs();
		return;
	}
	if (r.contains(QStringLiteral("certificate")))
	{
		statusBar()->showMessage(T(m_backend, "guest.cert.unknown").arg(r.value(QStringLiteral("certificate")).toObject().value(QStringLiteral("host")).toString()), 8000);
		return;
	}
	if (r.contains(QStringLiteral("session")))
	{
		rebuildTabs();
		const auto s = std::uint64_t(r.value(QStringLiteral("session")).toInteger());
		if (auto* p = pane(s))
		{
			// Neue Sitzung nach vorn, auch wenn eine Übersicht oder ein anderer erhaltener Tab vorn war.
			for (QTabWidget* tabs : findChildren<QTabWidget*>())
				for (int i = 0; i < tabs->count(); ++i)
					if (tabs->widget(i)->isAncestorOf(p))
						tabs->setCurrentIndex(i);
			if (p->terminal())
			{
				const QSize c = contentSize(true);
				m_backend->resizeSession(s, c.width(), c.height());
			}
			p->view()->setFocus();
		}
		return;
	}
	if (r.contains(QStringLiteral("ambiguous")))
	{
		chooseCredential(promptTab(id), r.value(QStringLiteral("ambiguous")).toArray());
		return;
	}
	if (r.contains(QStringLiteral("hostkey")))
	{
		// Unbekannter Host-Schlüssel eines Sprungservers: nie still annehmen.
		const QJsonObject h = r.value(QStringLiteral("hostkey")).toObject();
		ConnectPromptTab* tab = promptTab(id);
		QJsonObject q{ { QStringLiteral("text"), h.value(QStringLiteral("text")) },
			           { QStringLiteral("detail"), h.value(QStringLiteral("detail")) },
			           { QStringLiteral("timeout_secs"), m_backend->questionTimeout() },
			           { QStringLiteral("info"), QJsonObject{ { QStringLiteral("kind"), QStringLiteral("hostkey") },
			                                                  { QStringLiteral("host"), h.value(QStringLiteral("host")) },
			                                                  { QStringLiteral("port"), h.value(QStringLiteral("port")) },
			                                                  { QStringLiteral("key_type"), h.value(QStringLiteral("key_type")) },
			                                                  { QStringLiteral("fingerprint"), h.value(QStringLiteral("fingerprint")) },
			                                                  { QStringLiteral("previous"), h.value(QStringLiteral("previous")) } } } };
		QPointer<ConnectPromptTab> guard(tab);
		tab->security()->showQuestion(0, q, [this, id, h, guard](bool accept) {
			if (guard)
				closeToolTab(guard);
			if (accept &&
			    m_backend->acceptHostKey(h.value(QStringLiteral("host")).toString(), std::uint32_t(h.value(QStringLiteral("port")).toInt()),
			                             h.value(QStringLiteral("key")).toString()))
			{
				const QSize px = contentSize(false);
				handleConnectResult(id, m_backend->connectTarget(id, QString(), false, px.width(), px.height()));
			}
		});
		return;
	}
	if (r.contains(QStringLiteral("missing")))
	{
		const QJsonObject m = r.value(QStringLiteral("missing")).toObject();
		QString title = m.value(QStringLiteral("title")).toString();
		m_bar->inform(T(m_backend, "missing.credential.text")
		                .arg(T(m_backend, "missing.credential"), title.isEmpty() ? m.value(QStringLiteral("name")).toString() : title,
		                     m.value(QStringLiteral("reason")).toString()));
		return;
	}
	m_bar->inform(r.value(QStringLiteral("error")).toString());
}

ConnectPromptTab* MainWindow::promptTab(const QString& target)
{
	for (auto* t : findChildren<ConnectPromptTab*>())
		if (t->targetId() == target)
		{
			m_tabs->setCurrentWidget(t);
			return t;
		}
	// Ziel-ID ohne angehängten Zugangsweg („<id>#<Weg>“).
	QString name = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), target.section(QLatin1Char('#'), 0, 0) } })).value(QStringLiteral("name")).toString();
	if (name.isEmpty())
		name = target;
	auto* t = new ConnectPromptTab(m_backend, target, name, m_tabs);
	addToolTab(t, name);
	return t;
}

void MainWindow::chooseCredential(ConnectPromptTab* tab, const QJsonArray& choices)
{
	QList<QPair<QString, QString>> items;
	for (const QJsonValue& c : choices)
	{
		const QJsonObject o = c.toObject();
		QString label = o.value(QStringLiteral("name")).toString();
		const QString title = o.value(QStringLiteral("title")).toString();
		if (!title.isEmpty())
			label += QStringLiteral(" (") + title + QLatin1Char(')');
		items.append({ label, o.value(QStringLiteral("name")).toString() });
	}
	const QString id = tab->targetId();
	QPointer<ConnectPromptTab> guard(tab);
	tab->setMessage(T(m_backend, "dialog.credential.text"));
	tab->bar()->choose(T(m_backend, "dialog.credential.title"), items, nullptr, T(m_backend, "dialog.credential.remember"),
	                   [this, id, guard](const QString& name, bool remember) {
		                   if (guard)
			                   closeToolTab(guard);
		                   const QSize px = contentSize(false);
		                   handleConnectResult(id, m_backend->connectTarget(id, name, remember, px.width(), px.height()));
	                   });
	connect(tab->bar(), &ActionBar::cancelled, tab, [this, guard] {
		if (guard)
			closeToolTab(guard);
	});
}

void MainWindow::addToolTab(ToolTab* t, const QString& title)
{
	connect(t, &ToolTab::indexChanged, this, [this] {
		reloadTree();
		reloadViews();
		// Offene Eigenschaften-Tabs: neue oder geänderte Anmeldungen in der Auswahlliste zeigen.
		for (auto* p : findChildren<PropertiesTab*>())
			p->refreshLogins();
	});
	connect(t, &ToolTab::status, this, [this](const QString& m) { statusBar()->showMessage(m, 8000); });
	connect(t, &ToolTab::closeRequested, this, [this, t] { closeToolTab(t); });
	m_tabs->setCurrentIndex(m_tabs->addTab(t, title));
}

void MainWindow::addToolTab(QWidget* t, const QString& title)
{
	// Tab einer Erweiterung (RjPluginTab in deren Bibliothek): Signale nach Namen verbinden.
	const QMetaObject* mo = t->metaObject();
	if (mo->indexOfSignal("indexChanged()") >= 0)
		connect(t, SIGNAL(indexChanged()), this, SLOT(pluginTabIndexChanged()));
	if (mo->indexOfSignal("status(QString)") >= 0)
		connect(t, SIGNAL(status(QString)), this, SLOT(pluginTabStatus(QString)));
	if (mo->indexOfSignal("closeRequested()") >= 0)
		connect(t, SIGNAL(closeRequested()), this, SLOT(pluginTabClose()));
	m_tabs->setCurrentIndex(m_tabs->addTab(t, title));
}

void MainWindow::pluginTabIndexChanged()
{
	reloadTree();
	reloadViews();
	for (auto* p : findChildren<PropertiesTab*>())
		p->refreshLogins();
}

void MainWindow::pluginTabStatus(const QString& m)
{
	statusBar()->showMessage(m, 8000);
}

void MainWindow::pluginTabClose()
{
	if (auto* w = qobject_cast<QWidget*>(sender()))
		closeToolTab(w);
}

void MainWindow::closeToolTab(QWidget* t)
{
	if (auto* tt = qobject_cast<ToolTab*>(t))
		return closeToolTab(tt);
	const int i = m_tabs->indexOf(t);
	if (i < 0)
		return;
	m_tabs->removeTab(i);
	QMetaObject::invokeMethod(t, "wipeSecrets");
	t->deleteLater();
}

RjHostBackend* MainWindow::hostBackend() const
{
	return rjHostBackend(m_backend);
}

RjPluginUi* MainWindow::pluginUi(const QString& id)
{
	for (RjPluginUi* p : rjPluginUis(m_backend))
		if (p->id() == id)
			return p;
	return nullptr;
}

void MainWindow::ask(const QString& text, const QStringList& buttons, const std::function<void(int)>& done)
{
	m_bar->ask(text, buttons, done);
}

void MainWindow::prompt(const QString& text, const QString& preset, const std::function<void(const QString&)>& done)
{
	m_bar->prompt(text, preset, done);
}

RjHostOverview* MainWindow::findOverview()
{
	return findChild<OverviewTab*>();
}

void MainWindow::closeToolTab(ToolTab* t)
{
	const int i = m_tabs->indexOf(t);
	if (i < 0)
		return;
	m_tabs->removeTab(i);
	t->wipeSecrets();
	t->deleteLater();
}

void MainWindow::openImport(const QString& path, const QJsonObject& report, const QString& summary)
{
	addToolTab(new ImportTab(m_backend, path, report, summary, m_tabs), T(m_backend, "import.title"));
}

void MainWindow::showProperties(const QString& id)
{
	openProperties(id);
}

QStringList MainWindow::expandedIdsForCheck() const
{
	QStringList ids;
	std::function<void(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			if (m_tree->isExpanded(i) && !i.data(RoleId).toString().isEmpty())
				ids << i.data(RoleId).toString();
			walk(i);
		}
	};
	walk(QModelIndex());
	ids.sort();
	return ids;
}

void MainWindow::expandFirstHypervisor()
{
	std::function<bool(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			if (i.data(RoleProtocol).toString() == QStringLiteral("proxmox"))
			{
				m_tree->expand(i);
				m_tree->setCurrentIndex(m_model->index(0, 0, i));
				return true;
			}
			if (walk(i))
				return true;
		}
		return false;
	};
	walk(QModelIndex());
}

void MainWindow::collapseUnloadedHypervisors()
{
	std::function<void(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			const QModelIndex first = m_model->index(0, 0, i);
			if (first.isValid() && first.data(RolePlaceholder).toBool())
				m_tree->collapse(i);
			else
				walk(i);
		}
	};
	walk(QModelIndex());
}

void MainWindow::loadGuests(const QModelIndex& index)
{
	QStandardItem* parent = m_model->itemFromIndex(index);
	if (!parent)
		return;
	const QString target = index.data(RoleId).toString();
	const QJsonObject r = parseObject(m_backend->guestsJson(target));
	if (r.contains(QStringLiteral("pending")))
	{
		// Abruf läuft im Hintergrund; das Ergebnis kommt als Ereignis.
		if (parent->rowCount() == 1 && parent->child(0)->data(RolePlaceholder).toBool())
			parent->child(0)->setText(QStringLiteral("Wird geladen …"));
		updateJobs();
		return;
	}
	applyGuests(target, r);
}

void MainWindow::applyGuests(const QString& target, QJsonObject r)
{
	QStandardItem* parent = nullptr;
	std::function<void(const QModelIndex&)> find = [&](const QModelIndex& p) {
		for (int row = 0; row < m_model->rowCount(p) && !parent; ++row)
		{
			const QModelIndex i = m_model->index(row, 0, p);
			if (i.data(RoleId).toString() == target && !i.data(RoleGuest).isValid() && !i.data(RoleFavLink).toBool())
				parent = m_model->itemFromIndex(i);
			else
				find(i);
		}
	};
	find(QModelIndex());
	if (!parent)
		return;
	const QString proto = parent->data(RoleProtocol).toString();
	if (r.contains(QStringLiteral("certificate")))
	{
		// Unbekanntes oder geändertes Zertifikat: nie still annehmen.
		const QJsonObject c = r.value(QStringLiteral("certificate")).toObject();
		const bool changed = !c.value(QStringLiteral("old")).isNull() && c.contains(QStringLiteral("old"));
		ConnectPromptTab* tab = promptTab(target);
		QJsonObject q{ { QStringLiteral("text"), T(m_backend, changed ? "guest.cert.changed" : "guest.cert.unknown").arg(c.value(QStringLiteral("host")).toString()) },
			           { QStringLiteral("timeout_secs"), m_backend->questionTimeout() },
			           { QStringLiteral("info"), QJsonObject{ { QStringLiteral("kind"), QStringLiteral("certificate") },
			                                                  { QStringLiteral("host"), c.value(QStringLiteral("host")) },
			                                                  { QStringLiteral("key_type"), QStringLiteral("X.509") },
			                                                  { QStringLiteral("fingerprint"), c.value(QStringLiteral("fingerprint")) },
			                                                  { QStringLiteral("previous"), c.value(QStringLiteral("old")) } } } };
		QPointer<ConnectPromptTab> guard(tab);
		const QPersistentModelIndex pidx(parent->index());
		tab->security()->showQuestion(0, q, [this, c, guard, pidx](bool accept) {
			if (guard)
				closeToolTab(guard);
			if (accept && pidx.isValid() &&
			    m_backend->acceptProxmoxCertificate(c.value(QStringLiteral("host")).toString(), c.value(QStringLiteral("fingerprint")).toString()))
				loadGuests(pidx);
		});
		return;
	}
	parent->removeRows(0, parent->rowCount());
	if (r.contains(QStringLiteral("error")))
	{
		auto* e = new QStandardItem(T(m_backend, "guest.error").arg(r.value(QStringLiteral("error")).toString()));
		e->setEditable(false);
		e->setData(true, RolePlaceholder);
		e->setData(true, RoleFolder);
		parent->appendRow(e);
		return;
	}
	const QString consoleProto = proto == QStringLiteral("proxmox") ? QStringLiteral("vnc") : QStringLiteral("rdp");
	for (const QJsonValue& v : r.value(QStringLiteral("guests")).toArray())
	{
		const QJsonObject g = v.toObject();
		const QString state = g.value(QStringLiteral("state")).toString();
		auto* item = new QStandardItem(g.value(QStringLiteral("name")).toString());
		item->setEditable(false);
		item->setData(g.value(QStringLiteral("id")).toString(), RoleGuest);
		item->setData(target, RoleGuestTarget);
		item->setData(g.value(QStringLiteral("console_host")).toString(), RoleConsoleHost);
		item->setData(state, RoleGuestState);
		item->setData(true, RoleFolder);
		item->setIcon(protocolIcon(consoleProto, state == QStringLiteral("running") ? QStringLiteral("green") : QStringLiteral("grey"), false));
		item->setToolTip(g.value(QStringLiteral("detail")).toString() + QStringLiteral("\n") + state);
		parent->appendRow(item);
	}
}

void MainWindow::openGuestConsole(const QModelIndex& i)
{
	const QSize px = contentSize(false);
	const QString target = i.data(RoleGuestTarget).toString();
	handleConnectResult(target, m_backend->guestConsole(target, i.data(RoleGuest).toString(), i.data(Qt::DisplayRole).toString(),
	                                                    i.data(RoleConsoleHost).toString(), px.width(), px.height()));
}

void MainWindow::guestAction(const QModelIndex& i, const QString& action, bool confirmed)
{
	if (action == QStringLiteral("stop") && !confirmed)
	{
		const QPersistentModelIndex pi(i);
		m_bar->ask(T(m_backend, "guest.stop.confirm").arg(i.data(Qt::DisplayRole).toString()), { T(m_backend, "guest.stop.button") }, [this, pi, action](int b) {
			if (b == 0 && pi.isValid())
				guestAction(pi, action, true);
		});
		return;
	}
	const QJsonObject r = parseObject(m_backend->guestAction(i.data(RoleGuestTarget).toString(), i.data(RoleGuest).toString(), action));
	if (r.contains(QStringLiteral("pending")))
		updateJobs();
	else if (r.contains(QStringLiteral("error")))
		m_bar->inform(r.value(QStringLiteral("error")).toString());
	else
		statusBar()->showMessage(T(m_backend, "guest.action.sent"), 4000);
}

/// Benutzername oder Kennwort eines Ziels in die Zwischenablage; ein Kennwort wird nach der
/// eingestellten Frist wieder gelöscht, aber nur, wenn noch derselbe Inhalt darin steht.
void MainWindow::copyCredential(const QString& id, const QString& kind)
{
	const QJsonObject r = parseObject(m_backend->credentialCopy(id, kind));
	if (r.contains(QStringLiteral("error")))
	{
		statusBar()->showMessage(r.value(QStringLiteral("error")).toString(), 8000);
		return;
	}
	const QString text = r.value(QStringLiteral("text")).toString();
	const bool password = kind == QLatin1String("password");
	auto* md = new QMimeData;
	md->setText(text);
	if (password)
	{
		md->setData(QStringLiteral("x-kde-passwordManagerHint"), "secret");
#ifdef Q_OS_WIN
		// Windows: Das Kennwort soll weder im Zwischenablageverlauf (Win+V) noch in der Cloud-Zwischenablage
		// und nicht in Überwachungsprogrammen landen. Qt reicht diese Formate unter ihrem Windows-Namen durch.
		const QByteArray zero(4, '\0'); // DWORD 0
		md->setData(QStringLiteral("application/x-qt-windows-mime;value=\"ExcludeClipboardContentFromMonitorProcessing\""), zero);
		md->setData(QStringLiteral("application/x-qt-windows-mime;value=\"CanIncludeInClipboardHistory\""), zero);
		md->setData(QStringLiteral("application/x-qt-windows-mime;value=\"CanUploadToCloudClipboard\""), zero);
#endif
	}
	QGuiApplication::clipboard()->setMimeData(md);
	const int secs = r.value(QStringLiteral("clear_secs")).toInt(30);
	if (!password)
	{
		statusBar()->showMessage(T(m_backend, "ctx.copy.user.done"), 5000);
		return;
	}
	if (secs <= 0)
	{
		statusBar()->showMessage(T(m_backend, "ctx.copy.password.keep"), 8000);
		return;
	}
	m_clipHash = QCryptographicHash::hash(text.toUtf8(), QCryptographicHash::Sha256);
	if (!m_clipTimer)
	{
		m_clipTimer = new QTimer(this);
		m_clipTimer->setSingleShot(true);
		connect(m_clipTimer, &QTimer::timeout, this, [this] {
			auto* cb = QGuiApplication::clipboard();
			if (QCryptographicHash::hash(cb->text().toUtf8(), QCryptographicHash::Sha256) == m_clipHash)
				cb->clear();
			m_clipHash.clear();
		});
	}
	m_clipTimer->start(secs * 1000);
	statusBar()->showMessage(T(m_backend, "ctx.copy.password.done").arg(secs), 8000);
}

void MainWindow::treeContextMenu(const QPoint& pos)
{
	const QModelIndex i = m_tree->indexAt(pos);
	if (!i.isValid())
	{
		// Freie Fläche (auch bei leerem Baum): Hinzufügen auf oberster Ebene anbieten.
		QMenu empty(this);
		buildAddMenu(&empty, QString());
		empty.addSeparator();
		addTreeStartActions(&empty);
		empty.exec(m_tree->viewport()->mapToGlobal(pos));
		return;
	}
	if (i.data(RoleMacro).toInt())
	{
		m_tree->setCurrentIndex(i);
		macroContextMenu(i, m_tree->viewport()->mapToGlobal(pos));
		return;
	}
	if (i.data(RoleFavFolder).toBool())
		return;
	if (i.data(RoleFavLink).toBool())
	{
		// Verweis im Ordner „Favoriten“: verbinden, im Baum zeigen, Eigenschaften, aus den Favoriten nehmen.
		m_tree->setCurrentIndex(i);
		const QString id = i.data(RoleId).toString();
		QMenu fm(this);
		fm.addAction(faIcon(QStringLiteral("plug")), T(m_backend, "fav.connect"), this, [this] { connectSelected(); });
		fm.addAction(faIcon(QStringLiteral("sitemap")), T(m_backend, "fav.show"), this, [this, id] { selectId(id); });
		fm.addAction(faIcon(QStringLiteral("pen-to-square")), T(m_backend, "menu.edit.properties").remove(QLatin1Char('&')) + QStringLiteral("\tF4"), this, [this, id] { openProperties(id); });
		fm.addSeparator();
		fm.addAction(faIcon(QStringLiteral("r-star")), T(m_backend, "ctx.favorite.off") + QStringLiteral("\tCtrl+B"), this, [this] { toggleFavorite(); });
		fm.exec(m_tree->viewport()->mapToGlobal(pos));
		return;
	}
	for (const auto& h : m_modules.contextMenu)
		if (h(i, pos))
			return;
	if (i.data(RoleLogin).toBool())
	{
		m_tree->setCurrentIndex(i);
		const QString id = i.data(RoleId).toString();
		QMenu lm(this);
		lm.addAction(T(m_backend, "login.edit"), this, [this, id] { openLogin(id, QString()); });
		lm.addSeparator();
		lm.addAction(T(m_backend, "ctx.up") + QStringLiteral("\tAlt+Up"), this, [this] { reorderSelected(-1); });
		lm.addAction(T(m_backend, "ctx.down") + QStringLiteral("\tAlt+Down"), this, [this] { reorderSelected(1); });
		lm.addAction(T(m_backend, "ctx.move") + QStringLiteral("\tCtrl+Shift+M"), this, [this] { moveSelected(); });
		lm.addSeparator();
		lm.addAction(T(m_backend, "login.delete"), this, [this, id] {
			const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("login_delete") }, { QStringLiteral("id"), id } }));
			if (r.contains(QStringLiteral("error")))
				return showEditResult(r);
			edit({ { QStringLiteral("op"), QStringLiteral("save") } });
			for (auto* t : findChildren<LoginTab*>())
				if (t->loginId() == id)
					closeToolTab(t);
			reloadTree();
		});
		lm.exec(m_tree->viewport()->mapToGlobal(pos));
		return;
	}
	if (!m_tree->selectionModel()->isSelected(i))
		m_tree->setCurrentIndex(i);
	const QString id = i.data(RoleId).toString();
	QMenu menu(this);
	if (i.data(RoleGuest).isValid())
	{
		const QPersistentModelIndex pi(i);
		menu.addAction(T(m_backend, "ctx.guest.console"), this, [this, pi] { openGuestConsole(pi); });
		menu.addSeparator();
		menu.addAction(T(m_backend, "ctx.guest.start"), this, [this, pi] { guestAction(pi, QStringLiteral("start")); });
		menu.addAction(T(m_backend, "ctx.guest.shutdown"), this, [this, pi] { guestAction(pi, QStringLiteral("shutdown")); });
		menu.addAction(T(m_backend, "ctx.guest.reboot"), this, [this, pi] { guestAction(pi, QStringLiteral("reboot")); });
		menu.addAction(T(m_backend, "ctx.guest.stop"), this, [this, pi] { guestAction(pi, QStringLiteral("stop")); });
		menu.exec(m_tree->viewport()->mapToGlobal(pos));
		return;
	}
	const QString proto = i.data(RoleProtocol).toString();
	if (proto == QStringLiteral("proxmox") || proto == QStringLiteral("hyperv"))
	{
		const QPersistentModelIndex pi(i);
		menu.addAction(T(m_backend, "ctx.guest.refresh"), this, [this, pi] {
			loadGuests(pi);
			m_tree->expand(pi);
		});
	}
	if (!i.data(RoleFolder).toBool())
	{
		menu.addAction(T(m_backend, "ctx.connect"), this, [this] { connectSelected(); });
		menu.addAction(T(m_backend, "ctx.tile"), this, [this] {
			const auto tab = currentTab();
			connectCurrent();
			const auto s = currentSession();
			if (tab && s && m_backend->tileSession(s, tab))
				rebuildTabs();
		});
	}
	if (!i.data(RoleFolder).toBool())
	{
		menu.addSeparator();
		menu.addAction(T(m_backend, "ctx.copy.user"), this, [this, id] { copyCredential(id, QStringLiteral("user")); });
		menu.addAction(T(m_backend, "ctx.copy.password"), this, [this, id] { copyCredential(id, QStringLiteral("password")); });
		menu.addSeparator();
	}
	const bool folder = i.data(RoleFolder).toBool();
	QMenu* addM = menu.addMenu(T(m_backend, "ctx.add"));
	buildAddMenu(addM, folder ? id : currentParentId());
	menu.addAction(T(m_backend, "ctx.rename") + QStringLiteral("\tF2"), this, [this] { renameSelected(); });
	menu.addAction(T(m_backend, "ctx.duplicate") + QStringLiteral("\tCtrl+D"), this, [this] { duplicateSelected(); });
	menu.addAction(T(m_backend, "ctx.move") + QStringLiteral("\tCtrl+Shift+M"), this, [this] { moveSelected(); });
	menu.addAction(T(m_backend, "ctx.delete") + QStringLiteral("\tEntf"), this, [this] { deleteSelected(); });
	{
		// Einträge der Erweiterungen: neutrales Puzzleteil (der Kern kennt ihre Namen nicht).
		const QList<QAction*> before = menu.actions();
		for (const auto& h : m_modules.treeMenu)
			h(menu, i, id, folder);
		for (QAction* a : menu.actions())
			if (!before.contains(a) && !a->isSeparator() && a->icon().isNull())
				a->setIcon(faIcon(QStringLiteral("puzzle-piece")));
	}
	const bool fav = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), id } })).value(QStringLiteral("favorite")).toBool();
	menu.addAction(T(m_backend, fav ? "ctx.favorite.off" : "ctx.favorite.on") + QStringLiteral("\tCtrl+B"), this, [this] { toggleFavorite(); });
	if (folder)
	{
		menu.addSeparator();
		menu.addAction(T(m_backend, "ctx.all.connect"), this, [this] { connectAll(QStringLiteral("connect")); });
		menu.addAction(T(m_backend, "ctx.all.reconnect"), this, [this] { connectAll(QStringLiteral("reconnect")); });
		menu.addAction(T(m_backend, "ctx.all.disconnect"), this, [this] { connectAll(QStringLiteral("disconnect")); });
	}
	menu.addSeparator();
	menu.addAction(T(m_backend, "ctx.certcheck"), this, [this] {
		// Ein laufender Prüf-Tab wird ersetzt, damit Ergebnisse eindeutig zugeordnet bleiben.
		if (auto* old = findChild<CertCheckTab*>())
			closeToolTab(old);
		addToolTab(new CertCheckTab(m_backend, selectedIds(), m_tabs), T(m_backend, "certcheck.title"));
	});
	menu.addAction(T(m_backend, "ctx.mass"), this, [this] {
		const QStringList ids = selectedIds();
		if (ids.isEmpty())
			return;
		// Ein Lauf zur Zeit: ein vorhandener Reiter wird ersetzt.
		if (auto* old = findChild<MassInventoryTab*>())
			closeToolTab(old);
		const QString title = ids.size() == 1 ? m_tree->currentIndex().data().toString() : T(m_backend, "mass.title.n").arg(ids.size());
		auto* t = new MassInventoryTab(m_backend, ids, title, m_tabs);
		connect(t, &MassInventoryTab::showTarget, this, [this](const QString& id) { selectId(id); });
		connect(t, &MassInventoryTab::finished, this, [this] { reloadTree(); });
		addToolTab(t, T(m_backend, "mass.tab"));
	});
	for (const auto& h : m_modules.selectionMenu)
		h(menu);
	menu.addAction(T(m_backend, "ctx.credcheck"), this, [this] {
		const QStringList ids = selectedIds();
		if (ids.isEmpty())
			return;
		// Ein Lauf zur Zeit: ein vorhandener Reiter wird ersetzt.
		if (auto* old = findChild<CredCheckTab*>())
			closeToolTab(old);
		const QString title = ids.size() == 1 ? m_tree->currentIndex().data().toString() : T(m_backend, "mass.title.n").arg(ids.size());
		auto* t = new CredCheckTab(m_backend, ids, title, m_tabs);
		connect(t, &CredCheckTab::showTarget, this, [this](const QString& id) { selectId(id); });
		connect(t, &CredCheckTab::editTarget, this, [this](const QString& id) {
			selectId(id);
			openProperties(id);
		});
		addToolTab(t, T(m_backend, "credcheck.tab"));
	});
	menu.addAction(T(m_backend, "ctx.credmerge"), this, [this] {
		const QStringList ids = selectedIds();
		if (!ids.isEmpty())
			openCredMerge(ids);
	});
	menu.addAction(T(m_backend, "ctx.merge"), this, [this] {
		const QStringList ids = selectedIds();
		if (!ids.isEmpty())
			openMerge(ids, ids.size() == 1 ? m_tree->currentIndex().data().toString() : T(m_backend, "mass.title.n").arg(ids.size()));
	});
	// Mehrere markierte Ziele: zu einem Ziel zusammenfassen, gleich welche Adresse.
	if (const QStringList sel = selectedTargets(m_tree, m_model); sel.size() > 1)
		menu.addAction(T(m_backend, "ctx.merge.manual"), this, [this, sel] { openMerge(sel, T(m_backend, "mass.title.n").arg(sel.size()), true); });
	QMenu* tasks = menu.addMenu(T(m_backend, "ctx.tasks"));
	tasks->addAction(T(m_backend, "ctx.tasks.updates"), this, [this] {
		const QStringList ids = selectedTargets(m_tree, m_model);
		if (!ids.isEmpty())
			openPatchday(ids);
	});
	menu.addAction(T(m_backend, "ctx.properties") + QStringLiteral("\tF4"), this, [this, id] { openProperties(id); });
	menu.addSeparator();
	addTreeStartActions(&menu);
	menu.exec(m_tree->viewport()->mapToGlobal(pos));
}

void MainWindow::addTreeStartActions(QMenu* menu)
{
	// Aufklappzustand des Baums als Start (je Schlüsseldatenbank) speichern bzw. auf die Vorgabe zurücksetzen.
	menu->addAction(T(m_backend, "ctx.tree.start.save"), this, [this] {
		QJsonArray ids;
		std::function<void(const QModelIndex&)> collect = [&](const QModelIndex& parent) {
			for (int r = 0; r < m_model->rowCount(parent); ++r)
			{
				const QModelIndex i = m_model->index(r, 0, parent);
				if (m_tree->isExpanded(i) && i.data(RoleFolder).toBool() && !i.data(RoleId).toString().isEmpty())
					ids.append(i.data(RoleId).toString());
				collect(i);
			}
		};
		collect(QModelIndex());
		const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("set_tree_start") }, { QStringLiteral("ids"), ids } }));
		statusBar()->showMessage(r.contains(QStringLiteral("error")) ? r.value(QStringLiteral("error")).toString() : T(m_backend, "status.tree.start.saved").arg(ids.size()), 8000);
	});
	menu->addAction(T(m_backend, "ctx.tree.start.reset"), this, [this] {
		const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("set_tree_start") }, { QStringLiteral("ids"), QJsonValue() } }));
		statusBar()->showMessage(r.contains(QStringLiteral("error")) ? r.value(QStringLiteral("error")).toString() : T(m_backend, "status.tree.start.reset"), 8000);
	});
}

void MainWindow::tabContextMenu(QTabWidget* tabs, const QPoint& pos)
{
	const int idx = tabs->tabBar()->tabAt(pos);
	if (idx < 0)
		return;
	const auto tab = tabs->widget(idx)->property("tab").toULongLong();
	QMenu menu(this);
	if (tabs == m_tabs)
	{
		// Schließen wie mit dem Kreuz am Tab (gleiche Schutzregeln, z. B. ungespeicherte Makros).
		QAction* close = menu.addAction(faIcon(QStringLiteral("xmark")), T(m_backend, "tab.close"), this, [this, idx] {
			if (idx < m_tabs->count())
				emit m_tabs->tabCloseRequested(idx);
		});
		close->setObjectName(QStringLiteral("tabClose"));
		QAction* all = menu.addAction(T(m_backend, "tab.closeall"), this, [this] {
			const int n = m_tabs->count();
			ask(T(m_backend, "tab.closeall.ask").arg(n), { T(m_backend, "tab.closeall.yes") }, [this](int b) {
				if (b != 0)
					return;
				// Von hinten, damit die Positionen der übrigen Tabs gültig bleiben.
				for (int i = m_tabs->count() - 1; i >= 0; --i)
					emit m_tabs->tabCloseRequested(i);
			});
		});
		all->setObjectName(QStringLiteral("tabCloseAll"));
		menu.addSeparator();
	}
	if (tabs == m_tabs)
		menu.addAction(T(m_backend, "menu.view.detach"), this, [this, tab] {
			if (m_backend->detachTab(tab))
				rebuildTabs();
		});
	else
		menu.addAction(QStringLiteral("Zurück ins Hauptfenster"), this, [this, tab] {
			if (m_backend->attachTab(tab))
				rebuildTabs();
		});
	const char* modes[] = { "single", "one_by_two", "two_by_two", "free" };
	for (const char* m : modes)
	{
		const QString mode = QString::fromLatin1(m);
		menu.addAction(m_backend->text(QStringLiteral("menu.view.") + mode), this, [this, tab, mode] {
			if (m_backend->setArrangement(tab, mode))
				rebuildTabs();
		});
	}
	// Administrativer Tastaturmodus für jede RDP-Sitzung im Tab (vorübergehend, nur diese Sitzung).
	QList<SessionPane*> rdp;
	// SessionPane hat kein Q_OBJECT: über alle Kind-Widgets suchen.
	for (QWidget* c : tabs->widget(idx)->findChildren<QWidget*>())
		if (auto* p = dynamic_cast<SessionPane*>(c); p && p->protocol() == QLatin1String("rdp") && p->frame())
			rdp << p;
	if (!rdp.isEmpty())
	{
		menu.addSeparator();
		menu.setToolTipsVisible(true);
		for (SessionPane* p : rdp)
		{
			QAction* a = menu.addAction(rdp.size() == 1 ? T(m_backend, "adminkeys.menu") : T(m_backend, "adminkeys.menu.named").arg(p->title()));
			a->setCheckable(true);
			a->setChecked(p->frame()->adminKeys());
			a->setToolTip(T(m_backend, "adminkeys.tip"));
			QPointer<SessionPane> guard(p);
			connect(a, &QAction::triggered, this, [this, guard](bool on) {
				if (guard)
					setAdminKeys(guard, on);
			});
		}
	}
	menu.exec(tabs->tabBar()->mapToGlobal(pos));
}

void MainWindow::setAdminKeys(SessionPane* pane, bool on, const QString& reason)
{
	FrameView* f = pane ? pane->frame() : nullptr;
	if (!f)
		return;
	auto call = [this, pane](bool enable) {
		const QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("admin_keys") }, { QStringLiteral("session"), qint64(pane->session()) }, { QStringLiteral("on"), enable } };
		return parseObject(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))));
	};
	if (on)
	{
		// Erst lokal einschalten (löst gedrückte Tasten noch im bisherigen Modus), dann im Backend.
		f->setAdminKeys(true);
		const QJsonObject r = call(true);
		if (r.contains(QStringLiteral("error")))
		{
			f->setAdminKeys(false);
			statusBar()->showMessage(r.value(QStringLiteral("error")).toString(), 10000);
		}
		else
		{
			const QString name = r.value(QStringLiteral("name")).toString();
			f->setAdminKeys(true, r.value(QStringLiteral("timeout_mins")).toInt(), name);
			statusBar()->showMessage(T(m_backend, "adminkeys.on").arg(name), 10000);
		}
	}
	else if (f->adminKeys())
	{
		call(false);
		f->setAdminKeys(false);
		statusBar()->showMessage(reason.isEmpty() ? T(m_backend, "adminkeys.off") : reason, 10000);
	}
	updateAdminIndicators();
}

void MainWindow::updateAdminIndicators()
{
	static const QString objName = QStringLiteral("rjAdminKeys");
	// Zusätzlich dauerhaft in der Fußzeile: bleibt sichtbar, auch wenn ein Stil das Symbol am Tab nicht zeigt.
	{
		QStringList active;
		for (SessionPane* p : std::as_const(m_panes))
			if (p->frame() && p->frame()->adminKeys())
				active << p->title();
		auto* foot = statusBar()->findChild<QLabel*>(QStringLiteral("rjAdminKeysFoot"));
		if (!foot)
		{
			foot = new QLabel(statusBar());
			foot->setObjectName(QStringLiteral("rjAdminKeysFoot"));
			foot->setStyleSheet(QStringLiteral("QLabel { color: #ffffff; background: #e07000; border-radius: 3px; padding: 0 6px; font-weight: bold; }"));
			statusBar()->addPermanentWidget(foot);
		}
		foot->setText(T(m_backend, "adminkeys.foot").arg(active.join(QStringLiteral(", "))));
		foot->setToolTip(T(m_backend, "adminkeys.tip"));
		foot->setVisible(!active.isEmpty());
	}
	const QColor orange(QStringLiteral("#e07000"));
	auto pixmap = [&](qreal dpr) {
		const QString svg = QStringLiteral("<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'><g fill='none' stroke='%1' stroke-width='2.6' stroke-linecap='round' stroke-linejoin='round'>"
		                                   "<rect x='3' y='8' width='26' height='17' rx='2.5'/><path d='M8 13h1M12 13h1M16 13h1M20 13h1M24 13h1M8 17h1M12 17h1M16 17h1M20 17h1M24 17h1M11 21h10'/></g></svg>")
		                        .arg(orange.name());
		QPixmap pm(QSize(16, 16) * dpr);
		pm.fill(Qt::transparent);
		QPainter p(&pm);
		QSvgRenderer(svg.toUtf8()).render(&p);
		pm.setDevicePixelRatio(dpr);
		return pm;
	};
	for (QWidget* top : QApplication::topLevelWidgets())
		for (QTabWidget* tw : top->findChildren<QTabWidget*>())
			for (int i = 0; i < tw->count(); ++i)
			{
				FrameView* on = nullptr;
				for (QWidget* c : tw->widget(i)->findChildren<QWidget*>())
					if (auto* p = dynamic_cast<SessionPane*>(c); p && p->frame() && p->frame()->adminKeys())
						on = p->frame();
				auto* existing = qobject_cast<QLabel*>(tw->tabBar()->tabButton(i, QTabBar::LeftSide));
				const bool ours = existing && existing->objectName() == objName;
				// Diagnose (RJ_KEYLOG=1): welcher Tab das Symbol bekommt.
				// Windows-Programm ohne Konsole: direkt auf stderr (qInfo ginge dort an den Debugger).
				if (on && qEnvironmentVariableIsSet("RJ_KEYLOG"))
				{
					std::fprintf(stderr, "[remotejoe-tasten] Symbol Tastaturmodus: Tab %d \"%s\", vorhanden %s, Tabbreite %d, Leiste sichtbar %s\n", i, qUtf8Printable(tw->tabText(i)), ours ? "ja" : "nein",
					             tw->tabBar()->tabRect(i).width(), tw->tabBar()->isVisible() ? "ja" : "nein");
					std::fflush(stderr);
				}
				if (!on)
				{
					if (ours)
					{
						tw->tabBar()->setTabButton(i, QTabBar::LeftSide, nullptr);
						existing->deleteLater();
					}
					continue;
				}
				QLabel* label = ours ? existing : new QLabel;
				label->setToolTip(T(m_backend, "adminkeys.badge").arg(on->adminLayout()));
				if (ours)
					continue;
				label->setObjectName(objName);
				const QPixmap pm = pixmap(tw->devicePixelRatioF());
				label->setPixmap(pm);
				label->setFixedSize(18, 16);
				tw->tabBar()->setTabButton(i, QTabBar::LeftSide, label);
				// Die ersten Sekunden nach dem Einschalten blinken, danach ruhig orange.
				const qint64 until = on->adminSince() + 4000;
				if (QDateTime::currentMSecsSinceEpoch() < until)
				{
					auto* blink = new QTimer(label);
					blink->setInterval(350);
					connect(blink, &QTimer::timeout, label, [label, blink, pm, until] {
						if (QDateTime::currentMSecsSinceEpoch() >= until)
						{
							label->setPixmap(pm);
							blink->stop();
							blink->deleteLater();
							return;
						}
						label->setPixmap(label->pixmap().isNull() ? pm : QPixmap());
					});
					blink->start();
				}
			}
}

void MainWindow::showUnlock(bool locked)
{
	m_overlay->open(m_backend->kdbxPath(), locked);
}

void MainWindow::showNewVault()
{
	m_overlay->openNew();
}

void MainWindow::openMacroTab(const QString& id)
{
	for (int i = 0; i < m_tabs->count(); ++i)
		if (auto* mt = qobject_cast<MacroTab*>(m_tabs->widget(i)); mt && mt->macroId() == id)
		{
			m_tabs->setCurrentIndex(i);
			return;
		}
	auto* t = new MacroTab(m_backend, id, m_tabs);
	const int idx = m_tabs->addTab(t, t->title());
	m_tabs->setCurrentIndex(idx);
	connect(t, &MacroTab::titleChanged, this, [this, t] {
		const int i = m_tabs->indexOf(t);
		if (i >= 0)
			m_tabs->setTabText(i, t->title());
	});
	connect(t, &MacroTab::indexChanged, this, [this] { reloadTree(); });
	auto closeIt = [this, t] {
		const int i = m_tabs->indexOf(t);
		if (i >= 0)
		{
			m_tabs->removeTab(i);
			t->deleteLater();
		}
		reloadTree();
	};
	connect(t, &MacroTab::saved, this, [this, closeIt] {
		statusBar()->showMessage(T(m_backend, "macro.saved"), 5000);
		closeIt();
	});
	connect(t, &MacroTab::discarded, this, closeIt);
	reloadTree();
}

void MainWindow::deleteMacro(const QString& id, const QString& name)
{
	m_bar->ask(T(m_backend, "macro.delete.ask").arg(name), { T(m_backend, "bar.delete.button") }, [this, id](int i) {
		if (i != 0)
			return;
		for (auto* mt : findChildren<MacroTab*>())
			if (mt->macroId() == id)
			{
				m_tabs->removeTab(m_tabs->indexOf(mt));
				mt->deleteLater();
			}
		showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("macro_delete") }, { QStringLiteral("id"), id } })));
		reloadTree();
		m_tree->setFocus();
	});
}

void MainWindow::macroContextMenu(const QModelIndex& i, const QPoint& globalPos)
{
	QMenu menu(this);
	if (i.data(RoleMacro).toInt() == 1)
	{
		menu.addAction(T(m_backend, "macro.shelf.new"), this, [this] {
			const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("macro_new") } }));
			if (r.contains(QStringLiteral("macro")))
			{
				m_macroShelfOpen = true;
				openMacroTab(r.value(QStringLiteral("macro")).toString());
			}
		});
	}
	else
	{
		const QString id = i.data(RoleId).toString();
		const QString name = i.data(Qt::DisplayRole).toString();
		menu.addAction(T(m_backend, "macro.edit"), this, [this, id] { openMacroTab(id); });
		menu.addAction(T(m_backend, "macro.duplicate"), this, [this, id] {
			const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("macro_duplicate") }, { QStringLiteral("id"), id } }));
			reloadTree();
			if (r.contains(QStringLiteral("macro")))
				openMacroTab(r.value(QStringLiteral("macro")).toString());
		});
		menu.addAction(T(m_backend, "macro.delete") + QStringLiteral("\tEntf"), this, [this, id, name] { deleteMacro(id, name); });
	}
	menu.exec(globalPos);
}

void MainWindow::macrosForScreenshot()
{
	const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("macro_new") } }));
	const QString id = r.value(QStringLiteral("macro")).toString();
	const QJsonArray steps{
		QJsonObject{ { QStringLiteral("step"), QStringLiteral("send") }, { QStringLiteral("text"), QStringLiteral("sudo -v{key:enter}") } },
		QJsonObject{ { QStringLiteral("step"), QStringLiteral("wait") },
		             { QStringLiteral("pattern"), QJsonObject{ { QStringLiteral("text"), QStringLiteral("Kennwort") } } },
		             { QStringLiteral("errors"), QJsonArray{ QJsonObject{ { QStringLiteral("text"), QStringLiteral("Falsches Kennwort") } } } },
		             { QStringLiteral("timeout_ms"), 10000 } },
		QJsonObject{ { QStringLiteral("step"), QStringLiteral("send") }, { QStringLiteral("text"), QStringLiteral("{cred:password}{key:enter}") } },
		QJsonObject{ { QStringLiteral("step"), QStringLiteral("send") }, { QStringLiteral("text"), QStringLiteral("apt list --upgradable{key:enter}") } } };
	edit({ { QStringLiteral("op"), QStringLiteral("update_macro") }, { QStringLiteral("id"), id }, { QStringLiteral("name"), QStringLiteral("Updates prüfen") }, { QStringLiteral("steps"), steps } });
	m_macroShelfOpen = true;
	reloadTree();
	openMacroTab(id);
	m_tree->scrollToBottom();
}

void MainWindow::terminalSettingsForScreenshot(const QString& scheme)
{
	m_backend->setTerminalSettings(QStringLiteral("{\"scheme\":\"%1\"}").arg(scheme));
	for (auto* t : findChildren<SettingsTab*>())
		closeToolTab(t);
	openTerminalSettings();
}

bool MainWindow::showWebTabForScreenshot()
{
	for (SessionPane* p : std::as_const(m_panes))
		if (p && p->isWeb())
			for (int i = 0; i < m_tabs->count(); ++i)
				if (m_tabs->widget(i)->isAncestorOf(p))
				{
					m_tabs->setCurrentIndex(i);
					if (p->frame())
						p->frame()->setFocus();
					return true;
				}
	return false;
}

void MainWindow::showEmptyTreeForScreenshot()
{
	m_model->clear();
	updateEmptyHint();
}

SessionPane* MainWindow::pane(std::uint64_t session)
{
	return m_panes.value(session, nullptr);
}

std::uint64_t MainWindow::currentTab() const
{
	QWidget* w = m_tabs->currentWidget();
	return w ? w->property("tab").toULongLong() : 0;
}

std::uint64_t MainWindow::currentSession() const
{
	for (auto it = m_panes.begin(); it != m_panes.end(); ++it)
		if (it.value()->isAncestorOf(QApplication::focusWidget()))
			return it.key();
	const auto tab = currentTab();
	for (const QJsonValue& t : m_workspace.value(QStringLiteral("tabs")).toArray())
		if (std::uint64_t(t.toObject().value(QStringLiteral("id")).toInteger()) == tab)
		{
			const QJsonArray s = t.toObject().value(QStringLiteral("sessions")).toArray();
			if (!s.isEmpty())
				return std::uint64_t(s.last().toInteger());
		}
	return 0;
}

QWidget* MainWindow::buildTabContent(const QJsonObject& tab)
{
	const QJsonArray sessions = tab.value(QStringLiteral("sessions")).toArray();
	const QJsonArray cells = tab.value(QStringLiteral("cells")).toArray();
	const QJsonObject info = m_workspace.value(QStringLiteral("sessions")).toObject();
	auto paneFor = [&](std::uint64_t s) {
		SessionPane* p = m_panes.value(s, nullptr);
		if (!p)
		{
			const QJsonObject i = info.value(QString::number(s)).toObject();
			p = new SessionPane(m_backend, s, i.value(QStringLiteral("title")).toString(), i.value(QStringLiteral("protocol")).toString());
			p->setMacroOpener([this](const QString& macroId) { openMacroTab(macroId); });
			p->setFixedTitle(i.value(QStringLiteral("way")).toBool());
			if (!i.value(QStringLiteral("state")).toString().isEmpty())
				p->setState(i.value(QStringLiteral("state")).toString(), QString());
			m_panes.insert(s, p);
		}
		return p;
	};
	// Zeilen aus den Zellen bilden; jede Zeile ist ein waagerechter Teiler.
	auto* vertical = new QSplitter(Qt::Vertical);
	vertical->setChildrenCollapsible(false);
	QMap<int, QSplitter*> rows;
	const bool single = tab.value(QStringLiteral("arrangement")).toString() == QLatin1String("single");
	for (int i = 0; i < sessions.size(); ++i)
	{
		const auto s = std::uint64_t(sessions[i].toInteger());
		SessionPane* p = paneFor(s);
		if (single && i > 0)
		{
			p->setParent(nullptr);
			p->hide();
			continue;
		}
		const int row = i < cells.size() ? cells[i].toObject().value(QStringLiteral("row")).toInt() : 0;
		if (!rows.contains(row))
		{
			auto* h = new QSplitter(Qt::Horizontal);
			h->setChildrenCollapsible(false);
			rows.insert(row, h);
			vertical->addWidget(h);
		}
		rows[row]->addWidget(p);
		p->show();
	}
	return vertical;
}

void MainWindow::rebuildTabs()
{
	m_workspace = parseObject(m_backend->workspaceJson());
	QHash<std::uint64_t, QJsonObject> tabs;
	for (const QJsonValue& t : m_workspace.value(QStringLiteral("tabs")).toArray())
		tabs.insert(std::uint64_t(t.toObject().value(QStringLiteral("id")).toInteger()), t.toObject());
	// Sitzungs-Widgets aus alten Containern lösen, damit sie nicht mit gelöscht werden.
	for (SessionPane* p : std::as_const(m_panes))
		p->setParent(nullptr);
	auto fill = [&](QTabWidget* target, const QJsonObject& window) {
		const auto active = std::uint64_t(window.value(QStringLiteral("active")).toInteger());
		// Erhaltene Tabs (Eigenschaften, Übersicht, …) bleiben hinter der Sitzung, hinter der sie standen; Sitzungs-Tabs
		// werden in der Reihenfolge des Arbeitsbereichs neu aufgebaut, eine neue Sitzung kommt damit ganz nach rechts.
		struct Kept
		{
			QWidget* w;
			QString text;
			std::uint64_t after; // Sitzungs-Tab davor (0 = ganz vorn)
		};
		QWidget* current = target->currentWidget();
		QList<Kept> editors;
		std::uint64_t anchor = 0;
		while (target->count())
		{
			QWidget* w = target->widget(0);
			const QString text = target->tabText(0);
			target->removeTab(0);
			// Alles außer Sitzungs-Tabs bleibt (Einstellungen, Übersicht, Prüfungen, Anmeldung, Makros, …); nur die
			// Abfrage der Zugangsdaten gehört zur Verbindung und entfällt mit ihr.
			if (!w->property("tab").isValid() && !dynamic_cast<ConnectPromptTab*>(w))
				editors.append({ w, text, anchor });
			else
			{
				anchor = w->property("tab").toULongLong();
				w->deleteLater();
			}
		}
		QSet<std::uint64_t> present;
		for (const QJsonValue& tv : window.value(QStringLiteral("tabs")).toArray())
			present.insert(std::uint64_t(tv.toInteger()));
		auto addKept = [&](std::uint64_t after) {
			for (const Kept& e : std::as_const(editors))
				if (e.after == after || (after == UINT64_MAX && e.after != 0 && !present.contains(e.after)))
					target->addTab(e.w, e.text);
		};
		addKept(0);
		int activeIndex = 0;
		for (const QJsonValue& tv : window.value(QStringLiteral("tabs")).toArray())
		{
			const auto id = std::uint64_t(tv.toInteger());
			const QJsonObject tab = tabs.value(id);
			QWidget* content = buildTabContent(tab);
			content->setProperty("tab", QVariant::fromValue<qulonglong>(id));
			const int i = target->addTab(content, tab.value(QStringLiteral("title")).toString());
			if (id == active)
				activeIndex = i;
			for (SessionPane* p : std::as_const(m_panes))
				if (p && content->isAncestorOf(p))
					applyPageTitle(p);
			// Symbol des Protokolls der ersten Sitzung im Tab.
			for (SessionPane* p : std::as_const(m_panes))
				if (p && content->isAncestorOf(p))
				{
					const QString proto = parseObject(m_backend->sessionInfo(p->session())).value(QStringLiteral("protocol")).toString();
					if (!proto.isEmpty())
						target->setTabIcon(i, protocolIcon(proto, QString(), false));
					break;
				}
			addKept(id);
		}
		// Stand die Sitzung davor nicht mehr im Arbeitsbereich (geschlossen): ans Ende.
		addKept(UINT64_MAX);
		target->setCurrentIndex(activeIndex);
		for (const Kept& e : std::as_const(editors))
			if (e.w == current)
				target->setCurrentWidget(e.w);
	};
	const QJsonArray windows = m_workspace.value(QStringLiteral("windows")).toArray();
	QSet<std::uint64_t> seen;
	for (int wi = 0; wi < windows.size(); ++wi)
	{
		const QJsonObject w = windows[wi].toObject();
		const auto wid = std::uint64_t(w.value(QStringLiteral("id")).toInteger());
		if (wi == 0)
		{
			fill(m_tabs, w);
			continue;
		}
		seen.insert(wid);
		QTabWidget* tw = m_windowTabs.value(wid, nullptr);
		if (!tw || !m_windows.value(wid))
		{
			auto* win = new QWidget(nullptr, Qt::Window);
			win->setAttribute(Qt::WA_DeleteOnClose);
			win->setWindowTitle(T(m_backend, "app.title"));
			win->resize(900, 650);
			auto* lay = new QVBoxLayout(win);
			lay->setContentsMargins(0, 0, 0, 0);
			tw = new RjTabWidget(win);
			tw->setDocumentMode(true);
			tw->tabBar()->setContextMenuPolicy(Qt::CustomContextMenu);
			configureTabBar(tw);
			connect(tw->tabBar(), &QTabBar::customContextMenuRequested, this, [this, tw](const QPoint& p) { tabContextMenu(tw, p); });
			lay->addWidget(tw);
			m_windows.insert(wid, win);
			m_windowTabs.insert(wid, tw);
			win->show();
		}
		fill(tw, w);
	}
	for (auto it = m_windows.begin(); it != m_windows.end();)
	{
		if (!seen.contains(it.key()))
		{
			if (it.value())
				it.value()->close();
			m_windowTabs.remove(it.key());
			it = m_windows.erase(it);
		}
		else
			++it;
	}
	updateStatus();
	updateSessionMarks();
	updateAdminIndicators();
}

QWidget* MainWindow::detachedWindow() const
{
	for (const auto& w : m_windows)
		if (w)
			return w;
	return nullptr;
}

void MainWindow::applyAppearance()
{
	const QJsonObject a = parseObject(m_backend->uiPrefsJson());
	FrameView::setFocusFrame(a.value(QStringLiteral("focus_frame")).toBool(true));
	applyTheme(qEnvironmentVariable("REMOTEJOE_THEME") == QLatin1String("dark"));
	applyTreePrefs();
	for (auto* w : findChildren<QWidget*>())
		if (dynamic_cast<FrameView*>(w))
			w->update();
}

void MainWindow::openExport()
{
	for (int i = 0; i < m_tabs->count(); ++i)
		if (qobject_cast<ExportTab*>(m_tabs->widget(i)))
		{
			m_tabs->setCurrentIndex(i);
			return;
		}
	auto* t = new ExportTab(m_backend, [this] { return selectedIds(); }, m_tabs);
	connect(t, &ToolTab::status, this, [this](const QString& text) { statusBar()->showMessage(text, 10000); });
	m_tabs->setCurrentIndex(m_tabs->addTab(t, T(m_backend, "export.title")));
}

void MainWindow::openAppearance()
{
	openSettings(QStringLiteral("appearance"));
}

void MainWindow::openTerminalSettings()
{
	openSettings(QStringLiteral("terminal"));
}

void MainWindow::openSettings(const QString& section)
{
	SettingsTab* t = nullptr;
	for (int i = 0; i < m_tabs->count() && !t; ++i)
		t = qobject_cast<SettingsTab*>(m_tabs->widget(i));
	if (t)
		t->reload();
	else
	{
		t = new SettingsTab(m_backend, m_tabs);
		connect(t, &SettingsTab::appearanceChanged, this, [this] { applyAppearance(); });
		connect(t, &SettingsTab::closeTabKeyChanged, this, [this](const QString& mode) { m_closeTabKey = mode; });
		connect(t, &SettingsTab::findKeyChanged, this, [this](const QString& mode) { m_findKey = mode; });
		connect(t, &SettingsTab::tabSwitchKeysChanged, this, [this](const QString& mode, const QString& prev, const QString& next) { applyTabSwitchKeys(mode, prev, next); });
		connect(t, &SettingsTab::treeChanged, this, [this] { reloadTree(); });
		connect(t, &SettingsTab::featuresChanged, this, [this] {
			if (auto* o = findChild<OverviewTab*>())
				o->refreshInventorySwitch();
		});
		connect(t, &ToolTab::status, this, [this](const QString& text) { statusBar()->showMessage(text, 6000); });
		m_tabs->addTab(t, T(m_backend, "settings.title"));
	}
	if (!section.isEmpty())
		t->showSection(section);
	m_tabs->setCurrentWidget(t);
}

void MainWindow::updateConnInfo()
{
	const auto s = currentSession();
	const QJsonObject i = s ? parseObject(m_backend->sessionInfo(s)) : QJsonObject();
	if (i.isEmpty())
	{
		m_connInfo->hide();
		return;
	}
	const QColor c = palette().color(QPalette::WindowText);
	if (c != m_ciColor)
	{
		m_ciColor = c;
		m_ciTargetIcon->setPixmap(svgPixmap(kSvgTarget, c, 22));
		m_ciSinceIcon->setPixmap(svgPixmap(kSvgClock, c, 22));
		m_ciUserIcon->setPixmap(svgPixmap(kSvgUser, c, 22));
	}
	const QString proto = i.value(QStringLiteral("protocol")).toString();
	m_ciProtoIcon->setPixmap(protocolIcon(proto, QString(), false).pixmap(22, 22));
	m_ciProto->setText(protocolName(proto));
	const int port = i.value(QStringLiteral("port")).toInt();
	const QString host = i.value(QStringLiteral("host")).toString();
	m_ciTarget->setText(port > 0 ? QStringLiteral("%1:%2").arg(host).arg(port) : host);
	const QString state = i.value(QStringLiteral("state")).toString();
	if (i.value(QStringLiteral("since")).isNull() || state != QLatin1String("connected"))
		m_ciSince->setText(m_backend->text(QStringLiteral("state.") + (state.isEmpty() ? QStringLiteral("connecting") : state)));
	else
	{
		const QDateTime start = QDateTime::fromSecsSinceEpoch(qint64(i.value(QStringLiteral("since")).toDouble()));
		const qint64 sec = qMax<qint64>(0, start.secsTo(QDateTime::currentDateTime()));
		QString dur;
		if (sec < 60)
			dur = T(m_backend, "ci.dur.s").arg(sec);
		else if (sec < 3600)
			dur = T(m_backend, "ci.dur.ms").arg(sec / 60).arg(sec % 60);
		else
			dur = T(m_backend, "ci.dur.hm").arg(sec / 3600).arg((sec % 3600) / 60);
		const QString when = start.date() == QDate::currentDate() ? start.toString(QStringLiteral("HH:mm")) : start.toString(QStringLiteral("dd.MM. HH:mm"));
		m_ciSince->setText(T(m_backend, "ci.since").arg(when, dur));
	}
	const QString user = i.value(QStringLiteral("user")).toString();
	m_ciUser->setText(user.isEmpty() ? T(m_backend, "ci.nouser") : T(m_backend, "ci.user").arg(user));
	m_connInfo->show();
}

void MainWindow::updateStatus()
{
	QString state;
	const auto s = currentSession();
	if (auto* p = m_panes.value(s, nullptr))
		state = p->title() + QStringLiteral(": ") + m_backend->text(QStringLiteral("state.") + (p->state().isEmpty() ? QStringLiteral("connecting") : p->state()));
	statusBar()->showMessage(T(m_backend, "status.sessions").arg(m_panes.size()) + (state.isEmpty() ? QString() : QStringLiteral(" — ") + state));
	updateConnInfo();
}

void MainWindow::poll()
{
	handleEvents(m_backend->poll());
	for (const auto s : std::as_const(m_dirtyTerminals))
		if (auto* p = pane(s); p && p->terminal())
			p->terminal()->refresh();
	m_dirtyTerminals.clear();
}

void MainWindow::applyPageTitle(SessionPane* p)
{
	// Titel der Webseite als Tab-Titel, wenn der Tab nur diese Sitzung zeigt.
	if (p->pageTitle().isEmpty())
		return;
	QList<QTabWidget*> all = findChildren<QTabWidget*>();
	for (auto& w : m_windows)
		if (w)
			all += w->findChildren<QTabWidget*>();
	for (QTabWidget* tabs : all)
		for (int i = 0; i < tabs->count(); ++i)
			if (tabs->widget(i)->isAncestorOf(p) &&
			    std::count_if(m_panes.cbegin(), m_panes.cend(), [&](SessionPane* o) { return o && tabs->widget(i)->isAncestorOf(o); }) == 1)
			{
				if (!p->fixedTitle())
					tabs->setTabText(i, p->pageTitle());
				tabs->setTabToolTip(i, p->title() + QStringLiteral(" — ") + p->pageTitle());
			}
}

void MainWindow::handleEvents(const QString& json)
{
	if (json.size() > 2)
	{
		for (const QJsonValue& v : QJsonDocument::fromJson(json.toUtf8()).array())
		{
			const QJsonObject e = v.toObject();
			const QString t = e.value(QStringLiteral("t")).toString();
			const auto s = std::uint64_t(e.value(QStringLiteral("s")).toInteger());
			SessionPane* p = pane(s);
			if (t == QLatin1String("state"))
			{
				if (p)
					p->setState(e.value(QStringLiteral("state")).toString(), e.value(QStringLiteral("detail")).toString());
				// Verbindung getrennt oder im Neuaufbau: der administrative Tastaturmodus endet.
				if (p && p->frame() && p->frame()->adminKeys() && e.value(QStringLiteral("state")).toString() != QLatin1String("connected"))
					setAdminKeys(p, false, T(m_backend, "adminkeys.off.state"));
				updateNotice();
				updateStatus();
				updateSessionMarks();
			}
			else if (t == QLatin1String("shared_frame"))
			{
				const auto fd = std::int64_t(e.value(QStringLiteral("fd")).toInteger(-1));
				if (p && p->frame())
					p->frame()->attach(fd, e.value(QStringLiteral("w")).toInt(), e.value(QStringLiteral("h")).toInt());
				else
					FrameView::closeHandle(fd);
			}
			else if (t == QLatin1String("frame") && p && p->frame())
				p->frame()->damage(e.value(QStringLiteral("x")).toInt(), e.value(QStringLiteral("y")).toInt(), e.value(QStringLiteral("w")).toInt(),
				                   e.value(QStringLiteral("h")).toInt());
			else if (t == QLatin1String("terminal"))
				m_dirtyTerminals.insert(s);
			else if (t == QLatin1String("question"))
			{
				// Rückfrage als Leiste im Tab der Sitzung; ohne Tab: ablehnen.
				const auto q = std::uint32_t(e.value(QStringLiteral("q")).toInt());
				if (p)
				{
					for (int i = 0; i < m_tabs->count(); ++i)
						if (m_tabs->widget(i)->isAncestorOf(p))
							m_tabs->setCurrentIndex(i);
					p->security()->showQuestion(int(q), e, [this, s, q](bool accept) { m_backend->answer(s, q, accept); });
				}
				else
					m_backend->answer(s, q, false);
			}
			else if (t == QLatin1String("question_closed"))
			{
				if (p)
					p->security()->closeQuestion(e.value(QStringLiteral("q")).toInt(), e.value(QStringLiteral("reason")).toString());
				if (e.value(QStringLiteral("reason")).toString() == QLatin1String("timeout"))
					statusBar()->showMessage(T(m_backend, "security.timed_out"), 8000);
			}
			else if (t == QLatin1String("key_warning"))
			{
				// Schlüsselfehler, über den ohne Rückfrage hinweggegangen wurde: rot in der Fußzeile – an der Sitzung,
				// bei Hintergrundabfragen eine Minute lang für das ganze Fenster.
				const QString text = e.value(QStringLiteral("text")).toString();
				statusBar()->showMessage(text, 15000);
				if (p)
					p->setNotice(text, true);
				else
				{
					m_keyWarning = text;
					QTimer::singleShot(60000, this, [this, text] {
						if (m_keyWarning == text)
						{
							m_keyWarning.clear();
							updateNotice();
						}
					});
				}
				updateNotice();
			}
			else if (t == QLatin1String("notice"))
			{
				if (p)
					p->setNotice(e.value(QStringLiteral("text")).toString());
				updateNotice();
				statusBar()->showMessage(e.value(QStringLiteral("text")).toString(), 8000);
			}
			else if (t == QLatin1String("clipboard"))
				QGuiApplication::clipboard()->setText(e.value(QStringLiteral("text")).toString());
			else if (t == QLatin1String("clip_files"))
			{
				// Vom Ziel empfangene Dateien liegen im Ablageordner; als Datei-URLs in die lokale Zwischenablage.
				QList<QUrl> urls;
				for (const QJsonValue& v : e.value(QStringLiteral("paths")).toArray())
					urls << QUrl::fromLocalFile(v.toString());
				auto* md = new QMimeData;
				md->setUrls(urls);
				QGuiApplication::clipboard()->setMimeData(md);
				const QString msg = T(m_backend, "clipboard.files.received").arg(urls.size()).arg(e.value(QStringLiteral("dir")).toString());
				if (p)
					p->setNotice(msg);
				updateNotice();
				statusBar()->showMessage(msg, 10000);
			}
			else if (t == QLatin1String("clipboard_read_request"))
			{
				// Standard ist Ablehnen; Erlauben nur ausdrücklich.
				if (p)
					p->askBar()->ask(T(m_backend, "clipboard.read_request"), { T(m_backend, "clipboard.read_allow") }, [this, s](int b) {
						m_backend->clipboardReadAnswer(s, b == 0, b == 0 ? QGuiApplication::clipboard()->text() : QString());
					});
				else
					m_backend->clipboardReadAnswer(s, false, QString());
			}
			else if (t == QLatin1String("macro_done") && p)
				p->setMacroResult(e.value(QStringLiteral("message")).toString(), e.value(QStringLiteral("ok")).toBool());
			else if (t == QLatin1String("pointer") && p)
			{
				if (auto* fv = dynamic_cast<FrameView*>(p->view()))
					fv->setRemotePointer(e.value(QStringLiteral("kind")).toInt(), QByteArray::fromBase64(e.value(QStringLiteral("bgra")).toString().toLatin1()),
					                     e.value(QStringLiteral("w")).toInt(), e.value(QStringLiteral("h")).toInt(), e.value(QStringLiteral("hx")).toInt(),
					                     e.value(QStringLiteral("hy")).toInt());
			}
			else if (t == QLatin1String("title") && p)
				p->setToolTip(e.value(QStringLiteral("title")).toString());
			else if (t == QLatin1String("web_status") && p)
			{
				p->setWebStatus(e);
				applyPageTitle(p);
				// Lade- und Fehlerzustand in der Fußzeile.
				const QString error = e.value(QStringLiteral("error")).toString();
				if (!error.isEmpty())
					statusBar()->showMessage(T(m_backend, "web.status.error").arg(p->title(), error), 15000);
				else if (e.value(QStringLiteral("loading")).toBool())
					statusBar()->showMessage(T(m_backend, "web.status.loading").arg(p->title()).arg(e.value(QStringLiteral("progress")).toInt()), 5000);
				else
					statusBar()->showMessage(T(m_backend, "web.status.loaded").arg(p->title()), 3000);
			}
			else if (t == QLatin1String("fields_detected"))
			{
				for (auto* ed : findChildren<PropertiesTab*>())
					if (ed->nodeId() == e.value(QStringLiteral("target")).toString())
						ed->setDetectedFields(e);
			}
			else if (t == QLatin1String("web_menu") && p)
				p->showWebMenu(e);
			else if (t == QLatin1String("web_found") && p)
				p->setFindResult(e.value(QStringLiteral("found")).toBool());
			else if (t == QLatin1String("web_focus") && p && p->frame())
				p->frame()->webFocus(e.value(QStringLiteral("on")).toBool(), e.value(QStringLiteral("forward")).toBool());
			else if (t == QLatin1String("web_key") && p)
				FrameView::triggerShortcut(this, e.value(QStringLiteral("vk")).toInt(), e.value(QStringLiteral("ctrl")).toBool(),
				                           e.value(QStringLiteral("alt")).toBool(), e.value(QStringLiteral("shift")).toBool());
			else if (t == QLatin1String("web_download") && p)
			{
				// Download einer Web-Sitzung (Windows): Fortschritt und Ergebnis in der Fußzeile, Fehler zusätzlich als Hinweis.
				const QString state = e.value(QStringLiteral("state")).toString();
				const QString name = e.value(QStringLiteral("name")).toString();
				const QString dir = QDir::toNativeSeparators(e.value(QStringLiteral("dir")).toString());
				const QString error = e.value(QStringLiteral("error")).toString();
				if (state == QLatin1String("started"))
					statusBar()->showMessage(T(m_backend, "web.download.started").arg(p->title(), name), 5000);
				else if (state == QLatin1String("progress"))
				{
					const qlonglong total = e.value(QStringLiteral("total")).toVariant().toLongLong();
					const qlonglong got = e.value(QStringLiteral("received")).toVariant().toLongLong();
					statusBar()->showMessage(T(m_backend, "web.download.progress").arg(p->title(), name).arg(total > 0 ? int(got * 100 / total) : 0), 3000);
				}
				else if (state == QLatin1String("done"))
				{
					const bool opened = e.value(QStringLiteral("opened")).toBool();
					const QString text = !error.isEmpty() ? T(m_backend, "web.download.open_failed").arg(p->title(), name, dir, error)
					                     : opened      ? T(m_backend, "web.download.opened").arg(p->title(), name, dir)
					                                   : T(m_backend, "web.download.done").arg(p->title(), name, dir);
					statusBar()->showMessage(text, 15000);
					// Wohin die Datei gekommen ist, soll stehen bleiben (Hinweiszeile), nicht nur kurz erscheinen.
					p->setNotice(text);
					updateNotice();
				}
				else if (state == QLatin1String("failed"))
				{
					const QString text = T(m_backend, "web.download.failed").arg(p->title(), name, error);
					statusBar()->showMessage(text, 15000);
					p->setNotice(text);
					updateNotice();
				}
			}
			else if (t == QLatin1String("tree_changed"))
				reloadTree();
			else if (t == QLatin1String("unlocked"))
			{
				m_overlay->hide();
				for (auto& w : m_windows)
					if (w)
						w->show();
				reloadViews();
				// Erstes Entsperren: Startzustand bzw. Vorgabe des Baums anwenden (vorher war der Baum leer).
				if (m_firstUnlock)
				{
					m_firstUnlock = false;
					m_treeLoaded = false;
				}
				reloadTree();
				for (auto* ed : findChildren<PropertiesTab*>())
					ed->reload();
				statusBar()->showMessage(T(m_backend, "status.unlocked"), 5000);
				// Neue bzw. leere Schlüsseldatenbank: Hinweise zum ersten Ziel (nach dem Aufbau des Baums).
				QTimer::singleShot(500, this, [this] { showFirstHints(); });
				// Lizenz aus der Schlüsseldatenbank übernommen: Hinweis mit Neustart (gilt erst danach).
				licenseNoticeAfterUnlock();
				// Geteilte Zugangsdaten (z. B. aus früheren Duplikaten): Hinweis mit Verweis auf die Prüfroutine.
				QTimer::singleShot(0, this, [this] {
					const int n = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("shared_list") } })).value(QStringLiteral("groups")).toArray().size();
					if (n == 0)
						return;
					QMessageBox box(QMessageBox::Information, T(m_backend, "shared.head"), (n == 1 ? T(m_backend, "shared.unlock.one") : T(m_backend, "shared.unlock").arg(n)), QMessageBox::NoButton, this);
					auto* now = box.addButton(T(m_backend, "shared.unlock.check"), QMessageBox::AcceptRole);
					box.addButton(T(m_backend, "shared.unlock.later"), QMessageBox::RejectRole);
					box.exec();
					if (box.clickedButton() == now)
						openSharedCheck();
				});
				if (const QString note = m_backend->keyringNote(); !note.isEmpty())
					statusBar()->showMessage(note, 12000);
#ifdef Q_OS_WIN
				if (!g_updateNotice.isEmpty())
				{
					// Eigenes Hinweisfenster: Meldungen der Fußzeile werden beim Entsperren gleich wieder überschrieben.
					auto* box = new QMessageBox(QMessageBox::Information, QStringLiteral("remotejoe"), g_updateNotice, QMessageBox::Ok, this);
					box->setAttribute(Qt::WA_DeleteOnClose);
					box->setWindowModality(Qt::NonModal);
					QTimer::singleShot(300, box, &QWidget::show);
					g_updateNotice.clear();
				}
#endif
			}
			else if (t == QLatin1String("unlock_failed"))
			{
				if (!m_overlay->isVisible())
					showUnlock(false);
				m_overlay->setBusy(false, e.value(QStringLiteral("reason")).toString());
			}
			else if (t == QLatin1String("connect_all_progress"))
				rebuildTabs(); // weitere Verbindungen gestartet: Tabs (und ihre Rückfragen) sofort zeigen
			else if (t == QLatin1String("connect_all_done"))
			{
				rebuildTabs();
				statusBar()->showMessage(T(m_backend, "status.connect_all.done"), 5000);
			}
			else if (t == QLatin1String("session_opened"))
			{
				m_jobTargets.remove(qulonglong(e.value(QStringLiteral("job")).toInteger()));
				handleConnectResult(QString(), QString::fromUtf8(QJsonDocument(QJsonObject{ { QStringLiteral("session"), e.value(QStringLiteral("s")) } }).toJson()));
			}
			else if (t == QLatin1String("job_failed"))
			{
				const QString target = m_jobTargets.take(qulonglong(e.value(QStringLiteral("job")).toInteger()));
				QJsonObject r;
				if (e.value(QStringLiteral("hostkey")).isObject())
					r.insert(QStringLiteral("hostkey"), e.value(QStringLiteral("hostkey")));
				else
					r.insert(QStringLiteral("error"), e.value(QStringLiteral("error")));
				if (target.isEmpty())
					statusBar()->showMessage(e.value(QStringLiteral("error")).toString(), 8000);
				else
					handleConnectResult(target, QString::fromUtf8(QJsonDocument(r).toJson()));
			}
			else if (t == QLatin1String("job_result"))
			{
				const QString kind = e.value(QStringLiteral("kind")).toString();
				const QJsonObject r = parseObject(e.value(QStringLiteral("json")).toString());
				// Module zuerst; erledigt eines das Ergebnis, ist es fertig.
				bool byModule = false;
				for (const auto& h : m_modules.jobResult)
					byModule = h(kind, e.value(QStringLiteral("target")).toString(), r) || byModule;
				if (byModule)
				{
				}
				else if (kind.startsWith(QLatin1String("update_")))
				{
					if (auto* u = findChild<UpdateTab*>())
						u->handleResult(kind, r);
					else if (kind == QLatin1String("update_check") && r.value(QStringLiteral("state")).toString() == QLatin1String("newer"))
						// Prüfung beim Start: nur ein Hinweis in der Fußzeile.
						statusBar()->showMessage(T(m_backend, "update.available").arg(r.value(QStringLiteral("found")).toObject().value(QStringLiteral("version")).toString()), 15000);
				}
				else if (kind == QLatin1String("plugin_fetch"))
				{
					// Erweiterung heruntergeladen (gilt mit dem nächsten Start), aktuell oder Fehler: Fußzeile und Lizenzseite.
					const QString id = r.value(QStringLiteral("id")).toString();
					const QString state = r.value(QStringLiteral("state")).toString();
					if (state == QLatin1String("installed"))
						statusBar()->showMessage(T(m_backend, "plugin.fetch.installed").arg(id, r.value(QStringLiteral("version")).toString()), 20000);
					else if (state == QLatin1String("uptodate"))
						statusBar()->showMessage(T(m_backend, "plugin.fetch.uptodate").arg(id, r.value(QStringLiteral("version")).toString()), 10000);
					else
						statusBar()->showMessage(T(m_backend, "plugin.fetch.error").arg(id, r.value(QStringLiteral("error")).toString()), 20000);
					for (auto* st : findChildren<SettingsTab*>())
						st->licenseResult(r);
				}
				else if (kind == QLatin1String("backup"))
				{
					// Sicherung der KDBX: nur Fehler melden; die Einstellungen zeigen den Stand.
					for (const auto& bt : r.value(QStringLiteral("targets")).toArray())
						if (const QJsonObject o = bt.toObject(); !o.value(QStringLiteral("ok")).toBool())
							statusBar()->showMessage(T(m_backend, "status.backup.failed").arg(QDir::toNativeSeparators(o.value(QStringLiteral("dir")).toString()), o.value(QStringLiteral("error")).toString()), 15000);
					for (auto* st : findChildren<SettingsTab*>())
						st->reloadBackup();
				}
				else if (kind.startsWith(QLatin1String("ov_")))
				{
					if (auto* o = findChild<OverviewTab*>())
						o->handleResult(kind, e.value(QStringLiteral("target")).toString(), r);
				}
				else if (kind == QLatin1String("guests"))
					applyGuests(e.value(QStringLiteral("target")).toString(), r);
				else if (kind == QLatin1String("mass_inventory"))
				{
					// Der Reiter fragt den Stand selbst ab; hier nur die Symbole im Baum nachziehen.
					reloadTree();
				}
				else if (kind == QLatin1String("cert_check"))
				{
					if (auto* c = findChild<CertCheckTab*>())
						c->handleResult(r);
				}
				else if (kind == QLatin1String("credcheck"))
				{
					// Der Reiter fragt den Stand selbst ab.
				}
				else if (kind == QLatin1String("cert_accept"))
				{
					if (auto* c = findChild<CertCheckTab*>())
						c->handleAccept(r);
				}
				else if (r.contains(QStringLiteral("error")))
					statusBar()->showMessage(T(m_backend, "edit.error").arg(r.value(QStringLiteral("error")).toString()), 8000);
				else if (kind == QLatin1String("guest_action"))
					statusBar()->showMessage(T(m_backend, "guest.action.sent"), 4000);
				else
					handleConnectResult(e.value(QStringLiteral("target")).toString(), e.value(QStringLiteral("json")).toString());
			}
			else if (t == QLatin1String("jobs_changed"))
				updateJobs();
			else if (t == QLatin1String("locked"))
			{
				lockNow(true);
				statusBar()->showMessage(T(m_backend, "status.locked.idle"));
			}
			else if (t == QLatin1String("resumed"))
				statusBar()->showMessage(T(m_backend, "status.resumed"), 8000);
		}
	}
}

void MainWindow::updateJobs()
{
	const QJsonArray jobs = QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("jobs") } }).toUtf8()).array();
	m_jobsBox->setVisible(!jobs.isEmpty());
	if (jobs.isEmpty())
		return;
	const QJsonObject first = jobs.first().toObject();
	QString text = T(m_backend, "jobs.running").arg(first.value(QStringLiteral("label")).toString()).arg(first.value(QStringLiteral("secs")).toInt());
	if (jobs.size() > 1)
		text += QStringLiteral(" · ") + T(m_backend, "jobs.more").arg(jobs.size());
	m_jobsLabel->setText(text);
}

// Lange Tab-Titel in der Mitte kürzen (Breite begrenzt das Stilblatt); der volle Titel erscheint als Hinweis.
void MainWindow::configureTabBar(QTabWidget* tabs)
{
	QTabBar* bar = tabs->tabBar();
	bar->setElideMode(Qt::ElideMiddle);
	bar->setUsesScrollButtons(true);
	// Keine Grundlinie: der Windows-Stil zeichnete sie im Dokumentmodus als graue und weiße Linie über die ganze
	// Tableiste (sah aus wie eine Leiste über den Tabs).
	bar->setDrawBase(false);
	bar->installEventFilter(this);
}

void MainWindow::updateLogo()
{
	if (!m_logo)
		return;
	const int margin = 32;
	m_logo->move(m_tabs->width() - m_logo->width() - margin, m_tabs->height() - m_logo->height() - margin);
	const bool show = m_tabs->count() == 0 && m_tabs->width() > m_logo->width() + 2 * margin && m_tabs->height() > m_logo->height() + 2 * margin;
	m_logo->setVisible(show);
	if (show)
		m_logo->raise();
}

bool MainWindow::focusInSession() const
{
	for (QWidget* w = QApplication::focusWidget(); w; w = w->parentWidget())
		if (dynamic_cast<SessionPane*>(w))
			return true;
	return false;
}

bool MainWindow::closeTabKeyActive() const
{
	if (m_closeTabKey == QLatin1String("never"))
		return false;
	return m_closeTabKey == QLatin1String("always") || !focusInSession();
}

SessionPane* MainWindow::focusedWebPane() const
{
	for (QWidget* w = QApplication::focusWidget(); w; w = w->parentWidget())
		if (auto* pane = dynamic_cast<SessionPane*>(w); pane && pane->isWeb())
			return pane;
	return nullptr;
}

bool MainWindow::findKeyActive() const
{
	if (m_findKey == QLatin1String("never"))
		return false;
	return m_findKey == QLatin1String("always") || !focusInSession();
}

bool MainWindow::tabSwitchKeyActive() const
{
	if (m_tabSwitchKey == QLatin1String("never"))
		return false;
	return m_tabSwitchKey == QLatin1String("always") || !focusInSession();
}

void MainWindow::applyTabSwitchKeys(const QString& mode, const QString& prev, const QString& next)
{
	m_tabSwitchKey = mode.isEmpty() ? QStringLiteral("outside") : mode;
	const bool off = m_tabSwitchKey == QLatin1String("never");
	// Strg+Bild↑/↓ wechseln ohnehin immer; dieselbe Tastenfolge ein zweites Mal machte beide mehrdeutig.
	auto seq = [off](const QString& s) {
		const QKeySequence k(s, QKeySequence::PortableText);
		const bool fixed = k == QKeySequence(QStringLiteral("Ctrl+PgUp")) || k == QKeySequence(QStringLiteral("Ctrl+PgDown"));
		return off || fixed ? QKeySequence() : k;
	};
	if (m_tabPrevShortcut)
		m_tabPrevShortcut->setKey(seq(prev));
	if (m_tabNextShortcut)
		m_tabNextShortcut->setKey(seq(next));
}

void MainWindow::stepTab(int step)
{
	const int n = m_tabs ? m_tabs->count() : 0;
	if (n < 2)
		return;
	m_tabs->setCurrentIndex(((m_tabs->currentIndex() + step) % n + n) % n);
}

bool MainWindow::eventFilter(QObject* o, QEvent* e)
{
	// Logo unten rechts: Klick öffnet die Projektseite.
	if (o == m_logo && e->type() == QEvent::MouseButtonRelease && static_cast<QMouseEvent*>(e)->button() == Qt::LeftButton)
	{
		QDesktopServices::openUrl(QUrl(QStringLiteral("https://github.com/joeMJ/remotejoe")));
		return true;
	}
	// Gelöste Stecknadel: Zielbereich folgt der Größe der Mitte; Verlassen startet das Zuklappen.
	if (o == m_center && e->type() == QEvent::Resize)
		placeFlyout();
	if (o == m_leftPane && !m_pinned)
	{
		if (e->type() == QEvent::Enter)
			m_flyCloseTimer.stop();
		else if (e->type() == QEvent::Leave && m_flyOpen && !m_leftPane->isAncestorOf(QApplication::focusWidget()))
			m_flyCloseTimer.start(m_flyCloseMs);
		else if (e->type() == QEvent::Leave && m_flyOpen)
			m_flyCloseTimer.start(m_flyCloseMs * 4);
		else if (e->type() == QEvent::KeyPress && static_cast<QKeyEvent*>(e)->key() == Qt::Key_Escape)
			closeFlyout();
	}
	if (o == m_search && (e->type() == QEvent::ShortcutOverride || e->type() == QEvent::KeyPress))
	{
		auto* k = static_cast<QKeyEvent*>(e);
		if (k->matches(QKeySequence::Undo) && !m_search->isUndoAvailable())
		{
			if (e->type() == QEvent::KeyPress)
				undo();
			e->accept();
			return e->type() == QEvent::KeyPress;
		}
	}
	if (m_tabs && o == m_tabs && e->type() == QEvent::Resize)
		updateLogo();
	// Strg+Umschalt+W soll die Sitzung erreichen, wenn es den Tab gerade nicht schließen darf.
	if (e->type() == QEvent::ShortcutOverride)
	{
		auto* k = static_cast<QKeyEvent*>(e);
		if (k->key() == Qt::Key_W && (k->modifiers() & ~Qt::KeypadModifier) == (Qt::ControlModifier | Qt::ShiftModifier) && !closeTabKeyActive())
		{
			e->accept();
			return true;
		}
		// Ebenso die Tastenfolgen für den Tab-Wechsel, wenn sie gerade nicht wechseln dürfen.
		if (!tabSwitchKeyActive() && m_tabPrevShortcut && m_tabNextShortcut)
		{
			const QKeySequence pressed(k->keyCombination());
			if ((!m_tabPrevShortcut->key().isEmpty() && pressed == m_tabPrevShortcut->key()) || (!m_tabNextShortcut->key().isEmpty() && pressed == m_tabNextShortcut->key()))
			{
				e->accept();
				return true;
			}
		}
		// Ebenso Strg+F, wenn es nicht in die Suche über dem Baum springen soll (Web-Sitzungen suchen selbst).
		if (k->key() == Qt::Key_F && (k->modifiers() & ~Qt::KeypadModifier) == Qt::ControlModifier && !focusedWebPane() && !findKeyActive())
		{
			e->accept();
			return true;
		}
	}
	if (m_compact && m_tabs && o == m_tabs->tabBar() && e->type() == QEvent::Resize)
		QTimer::singleShot(0, this, [this] { syncStripHeight(); });
	// Platzsparmodus: die freie Fläche der Tab-Leiste ist die Titelzeile (Ziehen, Doppelklick maximiert).
	// Ohne Tab hat die Leiste keine Fläche; dann gilt die Streifenhöhe oben im Tab-Widget selbst.
	if (m_compact && m_tabs && (o == m_tabs->tabBar() || o == m_tabs) && (e->type() == QEvent::MouseButtonPress || e->type() == QEvent::MouseButtonDblClick))
	{
		auto* me = static_cast<QMouseEvent*>(e);
		const QPoint pt = me->position().toPoint();
		const bool inStrip = o == m_tabs ? pt.y() < qMax(26, m_titleBar->controls()->height()) : m_tabs->tabBar()->tabAt(pt) < 0;
		if (me->button() == Qt::LeftButton && inStrip)
		{
			if (e->type() == QEvent::MouseButtonDblClick)
			{
				toggleMaximized(this);
				m_titleBar->updateState();
			}
			else if (windowHandle())
				windowHandle()->startSystemMove();
			return true;
		}
	}
	if (e->type() == QEvent::ToolTip)
		if (auto* bar = qobject_cast<QTabBar*>(o))
		{
			const int i = bar->tabAt(static_cast<QHelpEvent*>(e)->pos());
			if (i >= 0)
			{
				QString full = bar->tabToolTip(i);
				if (full.isEmpty())
					full = bar->tabText(i);
				QToolTip::showText(static_cast<QHelpEvent*>(e)->globalPos(), full.remove(QLatin1Char('&')), bar);
				return true;
			}
		}
	if (m_tree && o == m_tree->viewport() && e->type() == QEvent::Resize)
		updateEmptyHint();
	// Zeiger am Fensterrand (Größenänderung): Er wird nur dort umgestellt und sonst sofort zurückgenommen.
	// Ein am Fenster gesetzter Zeiger würde sonst von allen Kindern geerbt (Terminal, RDP-Bild).
	if (e->type() == QEvent::MouseMove && o->isWidgetType() && static_cast<QWidget*>(o)->window() == this)
		updateEdgeCursor(static_cast<QMouseEvent*>(e)->globalPosition().toPoint());
	else if ((e->type() == QEvent::Leave || e->type() == QEvent::WindowDeactivate) && o == this && m_edgeOverride)
		updateEdgeCursor(QPoint(-100000, -100000));
	switch (e->type())
	{
	case QEvent::KeyPress:
	case QEvent::MouseButtonPress:
	case QEvent::Wheel:
	case QEvent::MouseMove:
		// Höchstens einmal je Sekunde an den Leerlauf-Zeitgeber melden.
		if (m_activityClock.elapsed() - m_lastActivity > 1000)
		{
			m_lastActivity = m_activityClock.elapsed();
			m_backend->activity();
		}
		break;
	default:
		break;
	}
	return QMainWindow::eventFilter(o, e);
}

void MainWindow::onPrepareForSleep(bool sleeping)
{
	if (!sleeping)
		systemResumed();
}

void MainWindow::systemResumed()
{
	handleEvents(m_backend->systemSleep(false));
}

void MainWindow::lockNow(bool alreadyLocked)
{
	if (!alreadyLocked)
		m_backend->lock();
	// Abgerissene Fenster zeigen Sitzungsbilder: ebenfalls verbergen.
	for (auto& w : m_windows)
		if (w)
			w->hide();
	for (auto* ed : findChildren<PropertiesTab*>())
		ed->wipeSecrets();
	for (auto* st : findChildren<SnmpTab*>())
		st->wipeSecrets();
	for (auto* tt : findChildren<ToolTab*>())
		tt->wipeSecrets();
	reloadTree();
	statusBar()->showMessage(T(m_backend, "status.locked"));
	showUnlock(true);
}

void MainWindow::showCredentialChoiceForScreenshot()
{
	QJsonArray choices;
	choices.append(QJsonObject{ { QStringLiteral("name"), QStringLiteral("admin") }, { QStringLiteral("title"), QStringLiteral("Verwaltung Standort Nord") } });
	choices.append(QJsonObject{ { QStringLiteral("name"), QStringLiteral("user") }, { QStringLiteral("title"), QStringLiteral("Benutzerkonto") } });
	PropertiesTab* ed = openFirstTargetProperties();
	const QString first = ed ? ed->nodeId() : QString();
	if (ed)
	{
		m_tabs->removeTab(m_tabs->indexOf(ed));
		ed->deleteLater();
	}
	ConnectPromptTab* tab = promptTab(first);
	tab->setObjectName(QStringLiteral("credentialPrompt"));
	chooseCredential(tab, choices);
}

QWidget* MainWindow::showHostKeyQuestionForScreenshot(bool changed)
{
	if (m_panes.isEmpty())
		return nullptr;
	SessionPane* p = m_panes.begin().value();
	for (int i = 0; i < m_tabs->count(); ++i)
		if (m_tabs->widget(i)->isAncestorOf(p))
			m_tabs->setCurrentIndex(i);
	QJsonObject info{ { QStringLiteral("kind"), QStringLiteral("hostkey") },
		              { QStringLiteral("host"), QStringLiteral("srv01.example.test") },
		              { QStringLiteral("port"), 22 },
		              { QStringLiteral("key_type"), QStringLiteral("ssh-ed25519") },
		              { QStringLiteral("fingerprint"), QStringLiteral("SHA256:q3e5V0bS3vLz2k8l1cYxN4hG6tJ9mWfQ7rUaPdE0oKs") } };
	QString text = QStringLiteral("Der Host-Schlüssel von srv01.example.test:22 ist unbekannt. Prüfen Sie den Fingerabdruck, bevor Sie ihn annehmen.");
	if (changed)
	{
		info.insert(QStringLiteral("previous"), QStringLiteral("SHA256:Zm9vYmFyYmF6cXV4ZXhhbXBsZWZpbmdlcnByaW50MDE"));
		text = QStringLiteral("WARNUNG: Der Host-Schlüssel von srv01.example.test:22 hat sich geändert. Das kann auf einen Angriff hinweisen.");
	}
	p->security()->showQuestion(changed ? 2 : 1, QJsonObject{ { QStringLiteral("text"), text }, { QStringLiteral("info"), info }, { QStringLiteral("timeout_secs"), 120 } },
	                            [](bool) {});
	return p;
}

void MainWindow::detachTabAt(int index)
{
	if (QWidget* w = m_tabs->widget(index))
		if (m_backend->detachTab(w->property("tab").toULongLong()))
			rebuildTabs();
}

void MainWindow::paintEvent(QPaintEvent* e)
{
	if (m_round)
	{
		// Durchsichtiges Fenster: Fläche selbst füllen (sonst schienen Lücken zwischen den Teilen durch), im
		// Normalzustand als abgerundetes Rechteck mit feinem Rand und leichtem Schatten im Größenrand.
		QPainter p(this);
		p.setRenderHint(QPainter::Antialiasing);
		p.setCompositionMode(QPainter::CompositionMode_Source);
		p.fillRect(rect(), Qt::transparent);
		p.setCompositionMode(QPainter::CompositionMode_SourceOver);
		const QColor bg = palette().color(QPalette::Window);
		if (isMaximized() || isFullScreen())
			p.fillRect(rect(), bg);
		else
		{
			// Abgerundete Fläche bis an den Fensterrand, feiner Rand in der Trennfarbe.
			const qreal r = RoundCorners::kRadius;
			const QRectF inner = QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5);
			p.setBrush(bg);
			p.setPen(QPen(palette().color(QPalette::Mid), 1));
			p.drawRoundedRect(inner, r, r);
		}
	}
	else
		QMainWindow::paintEvent(e);
	if (firstPaintMs < 0)
		firstPaintMs = startClock.elapsed();
}

// Rand, an dem die Größe verändert wird (das Fenster hat keinen Systemrahmen).
static Qt::Edges edgesAt(const QWidget* w, const QPoint& p)
{
	Qt::Edges e;
	if (p.x() < kResizeBorder)
		e |= Qt::LeftEdge;
	if (p.x() >= w->width() - kResizeBorder)
		e |= Qt::RightEdge;
	if (p.y() < kResizeBorder)
		e |= Qt::TopEdge;
	if (p.y() >= w->height() - kResizeBorder)
		e |= Qt::BottomEdge;
	return e;
}

static Qt::CursorShape cursorFor(Qt::Edges e)
{
	if (e == (Qt::LeftEdge | Qt::TopEdge) || e == (Qt::RightEdge | Qt::BottomEdge))
		return Qt::SizeFDiagCursor;
	if (e == (Qt::RightEdge | Qt::TopEdge) || e == (Qt::LeftEdge | Qt::BottomEdge))
		return Qt::SizeBDiagCursor;
	if (e & (Qt::LeftEdge | Qt::RightEdge))
		return Qt::SizeHorCursor;
	if (e & (Qt::TopEdge | Qt::BottomEdge))
		return Qt::SizeVerCursor;
	return Qt::ArrowCursor;
}

void MainWindow::updateEdgeCursor(const QPoint& global)
{
	// Runde Ecken: Zeiger und Ziehen übernehmen die Greifzonen (RoundCorners).
	if (m_round)
		return;
#ifdef Q_OS_WIN
	// Windows: Zeiger an den Rändern stellt das System (WM_NCHITTEST, siehe WinFrame).
	Q_UNUSED(global);
	return;
#endif
	const Qt::Edges edges = isMaximized() || isFullScreen() ? Qt::Edges() : edgesAt(this, mapFromGlobal(global));
	if (edges)
	{
		if (m_edgeOverride)
			QGuiApplication::changeOverrideCursor(QCursor(cursorFor(edges)));
		else
			QGuiApplication::setOverrideCursor(QCursor(cursorFor(edges)));
		m_edgeOverride = true;
	}
	else if (m_edgeOverride)
	{
		QGuiApplication::restoreOverrideCursor();
		m_edgeOverride = false;
	}
}

void MainWindow::mouseMoveEvent(QMouseEvent* e)
{
	QMainWindow::mouseMoveEvent(e);
}

void MainWindow::mousePressEvent(QMouseEvent* e)
{
#ifdef Q_OS_WIN
	// Windows: Größenänderung über den nativen Rahmen (WinFrame).
	const bool ownResize = false;
#else
	const bool ownResize = true;
#endif
	if (ownResize && e->button() == Qt::LeftButton && !isMaximized() && !isFullScreen())
	{
		const Qt::Edges edges = edgesAt(this, e->position().toPoint());
		if (edges && windowHandle())
		{
			windowHandle()->startSystemResize(edges);
			e->accept();
			return;
		}
	}
	QMainWindow::mousePressEvent(e);
}

void MainWindow::changeEvent(QEvent* e)
{
	QMainWindow::changeEvent(e);
	if (e->type() == QEvent::WindowStateChange)
	{
		// Maximiert: kein Rand zum Ziehen. Windows: den Innenrand setzt WinFrame.
#ifndef Q_OS_WIN
		const int m = m_round || isMaximized() || isFullScreen() ? 0 : kResizeBorder;
		setContentsMargins(m, m, m, m);
#endif
		if (m_titleBar)
			m_titleBar->updateState();
	}
}

/// Beim Start: mit dem im Schlüsselbund gemerkten Hauptkennwort entsperren (nur wenn eingeschaltet).
void MainWindow::tryKeyringUnlock()
{
	if (m_backend->unlockFromKeyring())
	{
		m_overlay->setBusy(true, QString());
		statusBar()->showMessage(T(m_backend, "status.keyring.unlocking"));
	}
}

void MainWindow::forgetKeyring()
{
	statusBar()->showMessage(m_backend->keyringForget(), 10000);
}

void MainWindow::restoreWindowState()
{
	const QJsonObject s = parseObject(m_backend->windowStateJson());
	if (!s.value(QStringLiteral("saved")).toBool())
		return;
	const int w = s.value(QStringLiteral("width")).toInt();
	const int h = s.value(QStringLiteral("height")).toInt();
	if (w < 400 || h < 300)
		return;
	// Nie größer als der sichtbare Bereich des Bildschirms (Dock, Monitorwechsel).
	QSize size(w, h);
	if (QScreen* sc = QGuiApplication::primaryScreen())
	{
		const QRect a = sc->availableGeometry();
		size = size.boundedTo(a.size());
	}
	resize(size);
	// Unter Wayland darf eine Anwendung ihre Lage nicht selbst setzen; dort gilt nur die Größe.
	// Sonst nur, wenn die Lage auf einem vorhandenen Bildschirm liegt.
	const QPoint pos(s.value(QStringLiteral("x")).toInt(), s.value(QStringLiteral("y")).toInt());
	if (QGuiApplication::platformName() != QLatin1String("wayland") && QGuiApplication::screenAt(pos + QPoint(40, 40)))
		move(pos);
	const int tree = s.value(QStringLiteral("tree_width")).toInt();
	if (m_split && tree >= 100)
		m_split->setSizes({ tree, qMax(200, size.width() - tree) });
	if (s.value(QStringLiteral("maximized")).toBool())
		setWindowState(windowState() | Qt::WindowMaximized);
	// Platzsparvariante und ausgeklappter Baum setzt der Programmstart (app.cpp), nicht der gespeicherte Zustand.
	fitToScreen();
}

void MainWindow::saveWindowState()
{
	// Bei maximiertem Fenster die normale Größe merken, damit sie nach dem Verkleinern stimmt.
	const QRect g = (isMaximized() || isFullScreen()) ? normalGeometry() : geometry();
	QJsonObject o;
	o.insert(QStringLiteral("x"), g.x());
	o.insert(QStringLiteral("y"), g.y());
	o.insert(QStringLiteral("width"), g.width());
	o.insert(QStringLiteral("height"), g.height());
	o.insert(QStringLiteral("maximized"), isMaximized());
	// Bei eingeklapptem Baum bleibt die zuletzt gemerkte Breite erhalten.
	if (m_split && !m_split->sizes().isEmpty() && m_split->sizes().first() >= 100)
		m_treeWidth = m_split->sizes().first();
	o.insert(QStringLiteral("tree_width"), m_treeWidth);
	o.insert(QStringLiteral("compact"), m_compact);
	o.insert(QStringLiteral("tree_hidden"), m_treeHidden);
	m_backend->saveWindowState(QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Compact)));
}

void MainWindow::screenLayoutChanged()
{
	// Mehrere Signale beim Umstecken des Docks: erst nach kurzer Ruhe prüfen.
	m_fitTimer.start();
}

void MainWindow::fitToScreen()
{
	if (isMaximized() || isFullScreen())
		return;
	QScreen* sc = screen();
	const QRect frame = frameGeometry();
	// Liegt das Fenster auf keinem Bildschirm mehr (Monitor abgesteckt), auf den Hauptbildschirm holen.
	bool onScreen = false;
	for (QScreen* x : QGuiApplication::screens())
		if (x->availableGeometry().intersects(QRect(frame.center() - QPoint(20, 20), QSize(40, 40))))
		{
			sc = x;
			onScreen = true;
			break;
		}
	if (!onScreen || !sc)
		sc = QGuiApplication::primaryScreen();
	if (!sc)
		return;
	const QRect a = sc->availableGeometry();
	const QSize extra = frame.size() - size();
	const QSize maxSize = a.size() - extra;
	QSize target = size().boundedTo(maxSize).expandedTo(QSize(400, 300));
	if (target != size())
		resize(target);
	if (QGuiApplication::platformName() == QLatin1String("wayland"))
		return;
	QRect f = frameGeometry();
	if (!onScreen)
		f.moveTopLeft(a.topLeft());
	// Mindestens die Titelzeile muss im sichtbaren Bereich liegen.
	if (f.right() > a.right())
		f.moveRight(a.right());
	if (f.bottom() > a.bottom())
		f.moveBottom(a.bottom());
	if (f.left() < a.left())
		f.moveLeft(a.left());
	if (f.top() < a.top())
		f.moveTop(a.top());
	if (f.topLeft() != frameGeometry().topLeft())
		move(f.topLeft() + (pos() - frameGeometry().topLeft()));
}

void MainWindow::resetWindow()
{
	if (isMaximized() || isFullScreen())
		showNormal();
	QScreen* sc = screen() ? screen() : QGuiApplication::primaryScreen();
	const QRect a = sc ? sc->availableGeometry() : QRect(0, 0, 1280, 800);
	m_treeWidth = 280;
	setTreeVisible(true);
	setCompact(true);
	resize(QSize(1280, 800).boundedTo(a.size() - (frameGeometry().size() - size())));
	if (m_split)
		m_split->setSizes({ 280, qMax(200, width() - 280) });
	if (QGuiApplication::platformName() != QLatin1String("wayland"))
		move(a.center() - QPoint(width() / 2, height() / 2));
	statusBar()->showMessage(T(m_backend, "status.window.reset"), 6000);
}

void MainWindow::setTreeVisible(bool on)
{
	// Gelöste Stecknadel: F9 klappt den Zielbereich auf und zu; die Registerkarte „Ziele“ bleibt.
	if (!m_pinned)
	{
		on ? openFlyout() : closeFlyout();
		return;
	}
	if (!on && m_split && !m_split->sizes().isEmpty() && m_split->sizes().first() >= 100 && !m_treeHidden && m_leftInner->isVisible())
		m_treeWidth = m_split->sizes().first();
	m_treeHidden = !on;
	if (m_treeButton)
	{
		m_treeButton->setText(on ? QStringLiteral("\u00ab") : QStringLiteral("\u00bb"));
		const QString tip = T(m_backend, on ? "status.tree.hide" : "status.tree.show");
		m_treeButton->setToolTip(tip);
		m_treeButton->setAccessibleName(tip);
	}
	placeBrand();
}

void MainWindow::setCompact(bool on)
{
	if (m_compactSwitch && m_compactSwitch->isChecked() != on)
	{
		QSignalBlocker b(m_compactSwitch);
		m_compactSwitch->setChecked(on);
	}
	if (m_compactAction && m_compactAction->isChecked() != on)
		m_compactAction->setChecked(on);
	if (m_compact == on && m_titleBar->controls()->parentWidget() != m_titleBar)
		return;
	m_compact = on;
	// Niedrigere Tabs im Platzsparmodus (Stylesheet: QTabBar[compact="true"]).
	m_tabs->tabBar()->setProperty("compact", on);
	m_tabs->tabBar()->style()->unpolish(m_tabs->tabBar());
	m_tabs->tabBar()->style()->polish(m_tabs->tabBar());
	QWidget* top = m_menuBar->parentWidget();
	if (on)
	{
		top->hide();
		m_titleBar->controls()->setBurgerMenu(m_burgerMenu);
		m_titleBar->controls()->setBurgerVisible(true);
		m_tabs->setCornerWidget(m_titleBar->controls(), Qt::TopRightCorner);
		m_titleBar->controls()->show();
	}
	else
	{
		m_tabs->setCornerWidget(nullptr, Qt::TopRightCorner);
		m_brandSlotLay->removeWidget(m_titleBar->brand());
		m_titleBar->restoreParts();
		m_brandSlot->hide();
		top->show();
	}
	placeBrand();
	m_titleBar->updateState();
}

/// Baum und Logo-Streifen anordnen. Im Platzsparmodus steht Logo, Name und Version bei angepinntem Baum
/// oben in der linken Spalte; bei eingeklapptem Baum links neben den Tabs, und die Sitzungsfläche reicht
/// darunter bis zum Fensterrand.
void MainWindow::placeBrand()
{
	if (!m_leftPane || !m_split)
		return;
	// Gelöste Stecknadel: der Baum liegt nicht im Teiler (Logo wie bei eingeklapptem Baum neben den Tabs).
	const bool on = !m_treeHidden && m_pinned;
	if (!m_compact)
	{
		m_leftInner->setVisible(true);
		if (!m_pinned)
			return;
		m_leftPane->setMinimumWidth(0);
		m_leftPane->setMaximumWidth(QWIDGETSIZE_MAX);
		m_leftPane->setVisible(on);
		if (on)
			m_split->setSizes({ m_treeWidth, qMax(200, m_split->width() - m_treeWidth) });
		return;
	}
	BrandStrip* brand = m_titleBar->brand();
	brand->setStyleSheet(QStringLiteral("QWidget#brandStrip { background: palette(window); border: none; }"
	                                    "QWidget#brandStrip QLabel { background: transparent; }"));
	brand->setAttribute(Qt::WA_StyledBackground, true);
	if (m_pinned)
		m_leftPane->setVisible(on);
	m_leftInner->setVisible(true);
	if (on)
	{
		m_tabs->setCornerWidget(nullptr, Qt::TopLeftCorner);
		if (brand->parentWidget() != m_brandSlot)
		{
			brand->setParent(m_brandSlot);
			m_brandSlotLay->addWidget(brand);
		}
		m_brandSlot->show();
		m_split->setSizes({ m_treeWidth, qMax(200, m_split->width() - m_treeWidth) });
	}
	else
	{
		m_brandSlotLay->removeWidget(brand);
		m_brandSlot->hide();
		m_tabs->setCornerWidget(brand, Qt::TopLeftCorner);
	}
	brand->show();
	syncStripHeight();
	QTimer::singleShot(0, this, [this] { syncStripHeight(); });
}

/// Logo-Streifen und Fensterknöpfe so hoch wie die Tab-Leiste, damit alles auf einer Linie liegt.
void MainWindow::syncStripHeight()
{
	if (!m_compact)
		return;
	// Die Ecken-Widgets enden an der Oberkante der Seite; so hoch wie dieser Abstand liegen sie bündig zu den Tabs.
	const int bh = m_tabs->tabBar()->height();
	const int py = m_tabs->currentWidget() ? m_tabs->currentWidget()->mapTo(m_tabs, QPoint(0, 0)).y() : 0;
	const int h = qMax(26, py > 16 ? py : (bh > 16 ? bh : m_tabs->tabBar()->sizeHint().height()));
	bool changed = false;
	for (QWidget* w : { static_cast<QWidget*>(m_titleBar->brand()), static_cast<QWidget*>(m_titleBar->controls()) })
		if (w->height() != h || w->minimumHeight() != h)
		{
			w->setFixedHeight(h);
			changed = true;
		}
	// Die Ecken-Widgets werden erst bei einem neuen Layout der Tab-Leiste an die Höhe angepasst.
	if (changed)
	{
		QResizeEvent ev(m_tabs->size(), m_tabs->size());
		QApplication::sendEvent(m_tabs, &ev);
	}
}

/// Hinweis der vorderen Sitzung in der Mitte der Fußzeile (statt der orangen Zeile in der Sitzung).
void MainWindow::updateNotice()
{
	if (!m_noticeLabel)
		return;
	// Zustand (nicht „Verbunden“) und Hinweis der Sitzungen im vorderen Tab; schwere Fehler rot und fett.
	QStringList parts;
	bool severe = false;
	if (QWidget* cur = m_tabs->currentWidget())
	{
		QList<SessionPane*> here;
		for (SessionPane* p : std::as_const(m_panes))
			if (p && (p == cur || cur->isAncestorOf(p)))
				here.append(p);
		std::sort(here.begin(), here.end(), [](SessionPane* a, SessionPane* b) { return a->session() < b->session(); });
		for (SessionPane* p : here)
		{
			QString line = p->statusLine();
			if (line.isEmpty() && p->notice().isEmpty())
				continue;
			if (!p->notice().isEmpty())
				line += (line.isEmpty() ? QString() : QStringLiteral(" · ")) + p->notice();
			if (here.size() > 1)
				line = p->title() + QStringLiteral(": ") + line;
			severe = severe || p->severe();
			parts.append(line);
		}
	}
	if (!m_keyWarning.isEmpty())
	{
		parts.prepend(m_keyWarning);
		severe = true;
	}
	const QString text = parts.join(QStringLiteral("   |   "));
	m_noticeLabel->setProperty("severe", severe);
	m_noticeLabel->style()->unpolish(m_noticeLabel);
	m_noticeLabel->style()->polish(m_noticeLabel);
	m_noticeLabel->setText(text);
	m_noticeLabel->setToolTip(text);
}

void MainWindow::closeEvent(QCloseEvent* e)
{
	// Ungespeicherte Änderungen (zum Beispiel ein Import) gehen sonst beim Beenden verloren.
	if (m_backend->hasUnsavedChanges())
	{
		QMessageBox box(QMessageBox::Warning, T(m_backend, "close.unsaved.title"), T(m_backend, "close.unsaved.text"),
		                QMessageBox::NoButton, this);
		auto* save = box.addButton(T(m_backend, "close.unsaved.save"), QMessageBox::AcceptRole);
		auto* discard = box.addButton(T(m_backend, "close.unsaved.discard"), QMessageBox::DestructiveRole);
		box.addButton(T(m_backend, "close.unsaved.cancel"), QMessageBox::RejectRole);
		box.setDefaultButton(save);
		box.exec();
		if (box.clickedButton() == save)
		{
			const QString r = m_backend->save();
			if (r.startsWith(QStringLiteral("Speichern fehlgeschlagen")))
			{
				statusBar()->showMessage(r, 10000);
				e->ignore();
				return;
			}
		}
		else if (box.clickedButton() != discard)
		{
			e->ignore();
			return;
		}
	}
	// Eine Sicherung im Hintergrund (z. B. auf ein NAS) nicht abschneiden: höchstens 10 s warten.
	if (parseObject(edit({ { QStringLiteral("op"), QStringLiteral("backup_status") } })).value(QStringLiteral("busy")).toBool())
	{
		statusBar()->showMessage(T(m_backend, "backup.exit.wait"));
		statusBar()->repaint();
		QApplication::setOverrideCursor(Qt::WaitCursor);
		edit({ { QStringLiteral("op"), QStringLiteral("backup_wait") }, { QStringLiteral("ms"), 10000 } });
		QApplication::restoreOverrideCursor();
	}
	saveWindowState();
	for (auto& w : m_windows)
		if (w)
			w->close();
	// Reguläres Beenden: Startmarke des Absturzwächters entfernen.
	edit({ { QStringLiteral("op"), QStringLiteral("plugins_start_ok") } });
	QMainWindow::closeEvent(e);
}

// ------------------------------------------------------------- Bearbeiten

QString MainWindow::edit(const QJsonObject& cmd)
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

void MainWindow::showEditResult(const QJsonObject& r, const QString& okMessage)
{
	if (r.contains(QStringLiteral("error")))
		statusBar()->showMessage(T(m_backend, "edit.error").arg(r.value(QStringLiteral("error")).toString()), 8000);
	else if (!okMessage.isEmpty())
		statusBar()->showMessage(okMessage, 5000);
	reloadTree();
	for (auto* ed : findChildren<PropertiesTab*>())
		if (!ed->isDirty())
			ed->reload();
}

QString MainWindow::currentId() const
{
	const QModelIndex i = m_tree->currentIndex();
	if (!i.isValid() || i.data(RoleGuest).isValid() || i.data(RolePlaceholder).toBool() || i.data(RoleMacro).toInt())
		return QString();
	return i.data(RoleId).toString();
}

QString MainWindow::currentParentId() const
{
	const QModelIndex i = m_tree->currentIndex();
	if (!i.isValid() || i.data(RoleMacro).toInt())
		return QString();
	if (i.data(RoleFolder).toBool() && !i.data(RolePlaceholder).toBool())
		return i.data(RoleId).toString();
	return i.parent().isValid() ? i.parent().data(RoleId).toString() : QString();
}

QStringList MainWindow::selectedIds() const
{
	QStringList ids;
	for (const QModelIndex& i : m_tree->selectionModel()->selectedIndexes())
		if (!i.data(RoleGuest).isValid() && !i.data(RolePlaceholder).toBool() && !i.data(RoleMacro).toInt())
			ids << i.data(RoleId).toString();
	if (ids.isEmpty() && !currentId().isEmpty())
		ids << currentId();
	return ids;
}

void MainWindow::selectId(const QString& id)
{
	std::function<bool(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			if (i.data(RoleId).toString() == id && !i.data(RoleGuest).isValid() && !i.data(RoleFavLink).toBool())
			{
				m_tree->setCurrentIndex(i);
				m_tree->scrollTo(i);
				return true;
			}
			if (walk(i))
				return true;
		}
		return false;
	};
	walk(QModelIndex());
}

void MainWindow::buildAddMenu(QMenu* menu, const QString& parentId)
{
	const char* kinds[][2] = { { "folder", "add.folder" }, { "rdp", "add.rdp" },         { "ssh", "add.ssh" },     { "vnc", "add.vnc" },
		                       { "web", "add.web" },       { "proxmox", "add.proxmox" }, { "hyperv", "add.hyperv" }, { "psrp", "add.psrp" },
		                       { "task", "add.task" },     { "macro", "add.macro" } };
	auto run = [this, parentId](QJsonObject cmd) {
		if (!parentId.isEmpty())
			cmd.insert(QStringLiteral("parent"), parentId);
		const QJsonObject r = parseObject(edit(cmd));
		showEditResult(r);
		if (r.contains(QStringLiteral("id")))
		{
			const QString id = r.value(QStringLiteral("id")).toString();
			selectId(id);
			openProperties(id, QStringLiteral("general"));
		}
		else if ((r.contains(QStringLiteral("task")) || r.contains(QStringLiteral("macro"))))
			openProperties(parentId.isEmpty() ? QString() : parentId, QStringLiteral("tasks"));
	};
	for (const auto& h : m_modules.addMenu)
		h(menu, parentId);
	{
		// Anmeldeobjekt im gewählten Ordner (bzw. im Ordner des gewählten Ziels).
		QAction* a = menu->addAction(T(m_backend, "add.login"), this, [this, parentId] { openLogin(QString(), parentId); });
		a->setObjectName(QStringLiteral("add_login"));
		a->setIcon(protocolIcon(QStringLiteral("login"), QString(), false));
	}
	for (auto& k : kinds)
	{
		const QString kind = QString::fromLatin1(k[0]);
		if (kind == QLatin1String("task"))
			menu->addSeparator();
		QAction* a = menu->addAction(m_backend->text(QString::fromLatin1(k[1])), this, [run, kind] {
			run({ { QStringLiteral("op"), QStringLiteral("add") }, { QStringLiteral("kind"), kind } });
		});
		a->setObjectName(QStringLiteral("add_") + kind);
		a->setIcon(protocolIcon(kind, QString(), kind == QLatin1String("folder")));
		if ((kind == QLatin1String("task") || kind == QLatin1String("macro")) && parentId.isEmpty())
			a->setEnabled(true);
	}
	menu->addSeparator();
	QAction* bulk = menu->addAction(T(m_backend, "add.bulk"), this, [this, parentId] { openBulkAdd(parentId); });
	bulk->setObjectName(QStringLiteral("add_bulk"));
	QMenu* tpl = menu->addMenu(T(m_backend, "add.template"));
	const QJsonArray list = QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("templates") } }).toUtf8()).array();
	if (list.isEmpty())
		tpl->addAction(T(m_backend, "add.template.none"))->setEnabled(false);
	for (const QJsonValue& v : list)
	{
		const QJsonObject o = v.toObject();
		const QString tid = o.value(QStringLiteral("id")).toString();
		tpl->addAction(protocolIcon(o.value(QStringLiteral("protocol")).toString(), QString(), false),
		               o.value(QStringLiteral("name")).toString() + (o.value(QStringLiteral("is_default")).toBool() ? QStringLiteral(" ★") : QString()), this,
		               [run, tid] { run({ { QStringLiteral("op"), QStringLiteral("add") }, { QStringLiteral("template"), tid } }); });
	}
}

QMenu* MainWindow::popupAddMenu()
{
	auto* menu = new QMenu(this);
	menu->setObjectName(QStringLiteral("addMenu"));
	menu->setAttribute(Qt::WA_DeleteOnClose);
	buildAddMenu(menu, currentParentId());
	const QRect r = m_tree->visualRect(m_tree->currentIndex());
	menu->popup(m_tree->viewport()->mapToGlobal(r.isValid() ? r.bottomLeft() : QPoint(0, 0)));
	return menu;
}

bool MainWindow::isLogin(const QString& id) const
{
	bool hit = false;
	std::function<void(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent) && !hit; ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			if (i.data(RoleId).toString() == id && i.data(RoleLogin).toBool())
				hit = true;
			else
				walk(i);
		}
	};
	walk(QModelIndex());
	return hit;
}

void MainWindow::openCredMerge(const QStringList& scope)
{
	// Ein Tab; ein neuer Aufruf ersetzt ihn (anderer Bereich).
	if (auto* old = findChild<CredMergeTab*>())
		closeToolTab(old);
	auto* t = new CredMergeTab(m_backend, scope, m_tabs);
	connect(t, &CredMergeTab::showTarget, this, [this](const QString& id) { selectId(id); });
	addToolTab(t, T(m_backend, "credmerge.tab"));
}

void MainWindow::openFolderOverview(const QModelIndex& folder, bool activate)
{
	// Elemente direkt im Ordner, wie sie im Baum stehen (Symbol, Typ, Adresse); das Inventar holt der Tab.
	auto typeName = [this](const QString& kind) -> QString {
		static const QMap<QString, QString> names = { { QStringLiteral("rdp"), QStringLiteral("RDP") },        { QStringLiteral("ssh"), QStringLiteral("SSH") },
			                                          { QStringLiteral("vnc"), QStringLiteral("VNC") },        { QStringLiteral("web"), QStringLiteral("Web") },
			                                          { QStringLiteral("proxmox"), QStringLiteral("Proxmox") }, { QStringLiteral("hyperv"), QStringLiteral("Hyper-V") },
			                                          { QStringLiteral("psrp"), QStringLiteral("PowerShell") },
			                                          { QStringLiteral("task"), QStringLiteral("Aufgabe") },   { QStringLiteral("multi"), QStringLiteral("Mehrere Wege") } };
		for (RjPluginUi* p : rjPluginUis(m_backend))
			if (const QString n = p->kindName(kind); !n.isEmpty())
				return n;
		return names.value(kind, kind.toUpper());
	};
	QList<FolderOverviewTab::Item> items;
	for (int r = 0; r < m_model->rowCount(folder); ++r)
	{
		const QModelIndex c = m_model->index(r, 0, folder);
		if (c.data(RolePlaceholder).toBool() || c.data(RoleMacro).toInt() != 0 || c.data(RoleLogin).toBool() || !c.data(RoleVSwitch).toString().isEmpty() || c.data(RoleId).toString().isEmpty())
			continue;
		FolderOverviewTab::Item it;
		it.id = c.data(RoleId).toString();
		it.name = c.data(Qt::DisplayRole).toString();
		it.folder = c.data(RoleFolder).toBool();
		it.type = typeName(c.data(RoleIconKind).toString());
		it.host = c.data(Qt::ToolTipRole).toString().section(QLatin1Char('\n'), 0, 0);
		it.icon = c.data(Qt::DecorationRole).value<QIcon>();
		items << it;
	}
	QStringList path;
	for (QModelIndex p = folder; p.isValid(); p = p.parent())
		path.prepend(p.data(Qt::DisplayRole).toString());
	auto* t = findChild<FolderOverviewTab*>();
	if (!t)
	{
		t = new FolderOverviewTab(m_backend, m_tabs);
		connect(t, &FolderOverviewTab::targetClicked, this, [this](const QString& id, const QIcon& icon) {
			selectId(id);
			openOverview(QJsonObject{ { QStringLiteral("id"), id } }, icon);
		});
		connect(t, &FolderOverviewTab::targetActivated, this, [this](const QString& id) {
			selectId(id);
			connectSelected();
		});
		connect(t, &FolderOverviewTab::folderClicked, this, [this](const QString& id) {
			selectId(id);
			if (m_tree->currentIndex().isValid())
				openFolderOverview(m_tree->currentIndex());
		});
		addToolTab(t, QString());
	}
	t->showFolder(folder.data(RoleId).toString(), folder.data(Qt::DisplayRole).toString(), path.join(QStringLiteral(" › ")), items);
	const int i = m_tabs->indexOf(t);
	m_tabs->setTabText(i, t->title());
	if (activate)
		m_tabs->setCurrentWidget(t);
}

void MainWindow::connectWay(const QJsonObject& s, const QString& way)
{
	const QString id = s.value(QStringLiteral("id")).toString();
	if (id.isEmpty())
		return;
	selectId(id);
	const QString full = id + QLatin1Char('#') + way;
	const QSize px = contentSize(false);
	handleConnectResult(full, m_backend->connectTarget(full, QString(), false, px.width(), px.height()));
}

void MainWindow::openOverview(const QJsonObject& subject, const QIcon& icon)
{
	auto* t = findChild<OverviewTab*>();
	if (!t)
	{
		t = new OverviewTab(m_backend, m_tabs);
		connect(t, &OverviewTab::titleChanged, this, [this, t] {
			const int i = m_tabs->indexOf(t);
			if (i >= 0)
				m_tabs->setTabText(i, T(m_backend, "ov.title").arg(t->targetName()));
		});
		// „Verbinden“ in der Kopfkarte: wie ein Doppelklick auf den Eintrag im Baum.
		connect(t, &OverviewTab::connectRequested, this, [this](const QJsonObject& s) {
			const QModelIndex cur = m_tree->currentIndex();
			if (s.contains(QStringLiteral("guest")))
			{
				if (cur.isValid() && cur.data(RoleGuest).toString() == s.value(QStringLiteral("guest")).toString())
					openGuestConsole(cur);
				return;
			}
			selectId(s.value(QStringLiteral("id")).toString());
			connectSelected();
		});
		// Nach einer Inventarabfrage: Symbol im Baum nachziehen (z. B. neue Geräteart erkannt).
		connect(t, &OverviewTab::inventoryChanged, this, [this] { QTimer::singleShot(0, this, [this] { reloadTree(); }); });
		// Knopf eines Zugangswegs: Ziel wählen und über diesen Weg verbinden.
		connect(t, &OverviewTab::connectWayRequested, this, &MainWindow::connectWay);
		connect(t, &OverviewTab::statusMessage, this, [this](const QString& m) { statusBar()->showMessage(m, 8000); });
		addToolTab(t, T(m_backend, "ov.title").arg(QString()));
	}
	else
		m_tabs->setCurrentWidget(t);
	t->showSubject(subject, icon);
}

void MainWindow::openLogin(const QString& id, const QString& folder)
{
	if (!id.isEmpty())
		for (auto* t : findChildren<LoginTab*>())
			if (t->loginId() == id)
			{
				m_tabs->setCurrentWidget(t);
				return;
			}
	// Neues Objekt: Ordner bestimmen (gewähltes Ziel → sein Ordner).
	QString f = folder;
	if (id.isEmpty() && !f.isEmpty())
	{
		std::function<QModelIndex(const QModelIndex&)> find = [&](const QModelIndex& parent) -> QModelIndex {
			for (int r = 0; r < m_model->rowCount(parent); ++r)
			{
				const QModelIndex i = m_model->index(r, 0, parent);
				if (i.data(RoleId).toString() == f)
					return i;
				if (const QModelIndex x = find(i); x.isValid())
					return x;
			}
			return QModelIndex();
		};
		const QModelIndex i = find(QModelIndex());
		if (i.isValid() && !i.data(RoleFolder).toBool())
			f = i.parent().isValid() ? i.parent().data(RoleId).toString() : QString();
	}
	auto* t = new LoginTab(m_backend, id, f, m_tabs);
	t->setIcon(protocolIcon(QStringLiteral("login"), QString(), false));
	addToolTab(t, t->tabTitle());
	connect(t, &LoginTab::titleChanged, this, [this, t] {
		const int i = m_tabs->indexOf(t);
		if (i >= 0)
			m_tabs->setTabText(i, t->tabTitle());
	});
}

PropertiesTab* MainWindow::openProperties(const QString& id, const QString& category)
{
	if (!id.isEmpty() && isLogin(id))
	{
		openLogin(id, QString());
		return nullptr;
	}
	QString target = id;
	if (target.isEmpty())
	{
		// Wurzel: ID über den Editor ermitteln.
		target = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), QStringLiteral("00000000-0000-0000-0000-000000000000") } })).value(QStringLiteral("id")).toString();
		if (target.isEmpty())
			return nullptr;
	}
	for (int i = 0; i < m_tabs->count(); ++i)
		if (auto* ed = qobject_cast<PropertiesTab*>(m_tabs->widget(i)); ed && ed->nodeId() == target)
		{
			m_tabs->setCurrentIndex(i);
			if (!category.isEmpty())
				ed->selectCategory(category);
			return ed;
		}
	auto* ed = new PropertiesTab(m_backend, target, m_tabs);
	connect(ed, &PropertiesTab::openLogin, this, [this](const QString& lid, const QString& folder) { openLogin(lid, folder); });
	const int idx = m_tabs->addTab(ed, ed->title());
	m_tabs->setTabToolTip(idx, T(m_backend, "edit.cat.general"));
	connect(ed, &PropertiesTab::titleChanged, this, [this, ed] {
		const int i = m_tabs->indexOf(ed);
		if (i >= 0)
			m_tabs->setTabText(i, ed->title());
	});
	connect(ed, &PropertiesTab::indexChanged, this, [this, ed] {
		reloadTree();
		reloadViews();
		for (auto* other : findChildren<PropertiesTab*>())
			if (other != ed && !other->isDirty())
				other->reload();
	});
	connect(ed, &PropertiesTab::status, this, [this](const QString& t) { statusBar()->showMessage(t, 5000); });
	connect(ed, &PropertiesTab::saveClosed, this, [this, ed] {
		const int i = m_tabs->indexOf(ed);
		if (i >= 0)
		{
			m_tabs->removeTab(i);
			ed->deleteLater();
		}
	});
	m_tabs->setCurrentWidget(ed);
	if (!category.isEmpty())
		ed->selectCategory(category);
	return ed;
}

BulkAddTab* MainWindow::openBulkAdd(const QString& parentId)
{
	auto* t = new BulkAddTab(m_backend, parentId, m_tabs);
	addToolTab(t, T(m_backend, "bulk.title"));
	t->hosts()->setFocus();
	return t;
}

PropertiesTab* MainWindow::openFirstTargetProperties(const QString& protocol)
{
	std::function<QString(const QModelIndex&)> walk = [&](const QModelIndex& parent) -> QString {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			if (!i.data(RoleFolder).toBool() && i.data(RoleProtocol).toString() == protocol)
				return i.data(RoleId).toString();
			const QString f = walk(i);
			if (!f.isEmpty())
				return f;
		}
		return QString();
	};
	const QString id = walk(QModelIndex());
	selectId(id);
	return id.isEmpty() ? nullptr : openProperties(id);
}

void MainWindow::renameSelected()
{
	for (const auto& h : m_modules.rename)
		if (h(m_tree->currentIndex()))
			return;
	const QString id = currentId();
	if (id.isEmpty())
		return;
	const QString name = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), id } })).value(QStringLiteral("name")).toString();
	m_bar->prompt(T(m_backend, "bar.rename").arg(m_tree->currentIndex().data().toString()), name, [this, id](const QString& n) {
		showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("rename") }, { QStringLiteral("id"), id }, { QStringLiteral("name"), n } })));
		selectId(id);
		m_tree->setFocus();
	});
}

void MainWindow::moveSelected()
{
	if (!m_tree->currentIndex().data(RoleVSwitch).toString().isEmpty())
		return; // virtueller Switch: bleibt beim Standort
	const QString id = currentId();
	if (id.isEmpty())
		return;
	QJsonObject e;
	if (m_tree->currentIndex().data(RoleLogin).toBool())
	{
		// Anmeldeobjekt: Ordnerliste und bisheriger Ordner aus der Anmeldeliste.
		const QJsonObject l = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("logins") } }));
		e.insert(QStringLiteral("folders"), l.value(QStringLiteral("folders")));
		for (const QJsonValue& v : l.value(QStringLiteral("logins")).toArray())
			if (v.toObject().value(QStringLiteral("id")).toString() == id)
				e.insert(QStringLiteral("parent"), v.toObject().value(QStringLiteral("folder")));
	}
	else
		e = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), id } }));
	QList<QPair<QString, QString>> items;
	for (const QJsonValue& f : e.value(QStringLiteral("folders")).toArray())
		items.append({ f.toObject().value(QStringLiteral("path")).toString(), f.toObject().value(QStringLiteral("id")).toString() });
	m_bar->choose(T(m_backend, "bar.move").arg(m_tree->currentIndex().data().toString()), items, [this, id](const QString& parent) {
		showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("move") }, { QStringLiteral("id"), id }, { QStringLiteral("parent"), parent } })));
		selectId(id);
		m_tree->setFocus();
	});
	const int cur = m_bar->combo()->findData(e.value(QStringLiteral("parent")).toString());
	if (cur >= 0)
		m_bar->combo()->setCurrentIndex(cur);
}

void MainWindow::deleteSelected()
{
	for (const auto& h : m_modules.remove)
		if (h(m_tree->currentIndex()))
			return;
	const QStringList ids = selectedIds();
	if (ids.isEmpty())
		return;
	QString text = ids.size() == 1 ? T(m_backend, "bar.delete").arg(m_tree->currentIndex().data().toString()) : T(m_backend, "bar.delete.many").arg(ids.size());
	// Offene Sitzungen der betroffenen Ziele (auch in Unterordnern) werden beendet; darauf hinweisen.
	const QJsonObject preview = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("delete_preview") }, { QStringLiteral("ids"), QJsonArray::fromStringList(ids) } }));
	if (const int active = preview.value(QStringLiteral("active")).toInt(); active > 0)
		text += QLatin1Char(' ') + T(m_backend, "bar.delete.sessions").arg(active);
	m_bar->ask(text, { T(m_backend, "bar.delete.button") }, [this, ids](int i) {
		if (i != 0)
			return;
		// Offene Eigenschaften-Tabs gelöschter Objekte schließen.
		for (auto* ed : findChildren<PropertiesTab*>())
			if (ids.contains(ed->nodeId()))
			{
				m_tabs->removeTab(m_tabs->indexOf(ed));
				ed->deleteLater();
			}
		const QJsonObject result = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("delete") }, { QStringLiteral("ids"), QJsonArray::fromStringList(ids) } }));
		// Beendete Sitzungen: Ansichten und Tabs entfernen.
		const QJsonArray closed = result.value(QStringLiteral("closed_sessions")).toArray();
		for (const QJsonValue& s : closed)
			if (auto* p = m_panes.take(std::uint64_t(s.toInteger())))
				p->deleteLater();
		showEditResult(result);
		if (!closed.isEmpty())
			rebuildTabs();
		m_tree->setFocus();
	});
}

void MainWindow::duplicateSelected()
{
	if (!m_tree->currentIndex().data(RoleVSwitch).toString().isEmpty())
		return;
	const QString id = currentId();
	if (id.isEmpty())
		return;
	const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("duplicate") }, { QStringLiteral("id"), id } }));
	showEditResult(r);
	selectId(r.value(QStringLiteral("id")).toString());
}

void MainWindow::toggleFavorite()
{
	const QString id = currentId();
	if (id.isEmpty())
		return;
	const bool fav = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), id } })).value(QStringLiteral("favorite")).toBool();
	showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("favorite") }, { QStringLiteral("id"), id }, { QStringLiteral("on"), !fav } })),
	               T(m_backend, fav ? "ctx.favorite.off" : "ctx.favorite.on").remove(QLatin1Char('&')));
	selectId(id);
}

void MainWindow::reorderSelected(int delta)
{
	const QString id = currentId();
	if (id.isEmpty())
		return;
	showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("reorder") }, { QStringLiteral("id"), id }, { QStringLiteral("delta"), delta } })));
	selectId(id);
}

void MainWindow::undo()
{
	const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("undo") } }));
	if (r.contains(QStringLiteral("undone")))
		showEditResult(r, T(m_backend, "status.undone").arg(r.value(QStringLiteral("undone")).toString()));
	else
		statusBar()->showMessage(T(m_backend, "status.nothing_to_undo"), 4000);
	reloadViews();
}

void MainWindow::connectAll(const QString& mode)
{
	const QString id = currentId();
	if (id.isEmpty())
		return;
	if (mode == QLatin1String("connect"))
	{
		connectMany({ { QStringLiteral("id"), id } });
		return;
	}
	const QSize px = contentSize(false);
	const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("connect_all") },
	                                         { QStringLiteral("id"), id },
	                                         { QStringLiteral("mode"), mode },
	                                         { QStringLiteral("width"), px.width() },
	                                         { QStringLiteral("height"), px.height() } }));
	if (r.contains(QStringLiteral("error")))
		statusBar()->showMessage(r.value(QStringLiteral("error")).toString(), 8000);
	else if (mode == QLatin1String("connect"))
		statusBar()->showMessage(T(m_backend, "status.connect_all").arg(r.value(QStringLiteral("count")).toInt()).arg(4), 8000);
	rebuildTabs();
}

void MainWindow::reloadViews()
{
	const QString cur = m_viewBox->currentData().toString();
	m_viewBox->clear();
	m_viewBox->addItem(T(m_backend, "view.all"), QString());
	m_viewBox->addItem(T(m_backend, "view.favorites"), QStringLiteral("favorites"));
	for (const QJsonValue& v : QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("views") } }).toUtf8()).array())
		m_viewBox->addItem(v.toObject().value(QStringLiteral("name")).toString() + QStringLiteral(" (%1)").arg(v.toObject().value(QStringLiteral("count")).toInt()),
		                   v.toObject().value(QStringLiteral("id")).toString());
	const int i = m_viewBox->findData(cur);
	m_viewBox->setCurrentIndex(i < 0 ? 0 : i);
	if (i < 0 && !cur.isEmpty())
	{
		edit({ { QStringLiteral("op"), QStringLiteral("set_view") }, { QStringLiteral("view"), QString() } });
		reloadTree();
	}
}

void MainWindow::viewCommand(const QString& what)
{
	const QString view = m_viewBox->currentData().toString();
	const bool real = !view.isEmpty() && view != QLatin1String("favorites");
	if (what == QLatin1String("new") || what == QLatin1String("new_with"))
	{
		const QStringList ids = what == QLatin1String("new_with") ? selectedIds() : QStringList();
		m_bar->prompt(T(m_backend, "bar.view.new"), QString(), [this, ids](const QString& name) {
			showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("view_create") }, { QStringLiteral("name"), name }, { QStringLiteral("ids"), QJsonArray::fromStringList(ids) } })));
			reloadViews();
		});
	}
	else if (what == QLatin1String("rename") && real)
		m_bar->prompt(T(m_backend, "bar.view.rename"), m_viewBox->currentText().section(QStringLiteral(" ("), 0, 0), [this, view](const QString& name) {
			showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("view_rename") }, { QStringLiteral("view"), view }, { QStringLiteral("name"), name } })));
			reloadViews();
		});
	else if (what == QLatin1String("delete") && real)
		m_bar->ask(T(m_backend, "bar.view.delete").arg(m_viewBox->currentText()), { T(m_backend, "bar.delete.button") }, [this, view](int i) {
			if (i != 0)
				return;
			showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("view_delete") }, { QStringLiteral("view"), view } })));
			reloadViews();
		});
}

void MainWindow::saveAll(const QString& choice)
{
	QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("save") } };
	if (!choice.isEmpty())
		cmd.insert(QStringLiteral("choice"), choice);
	const QJsonObject r = parseObject(edit(cmd));
	if (r.contains(QStringLiteral("conflict")))
	{
		const QJsonObject c = r.value(QStringLiteral("conflict")).toObject();
		m_bar->ask(T(m_backend, "bar.conflict").arg(c.value(QStringLiteral("mine")).toInteger()).arg(c.value(QStringLiteral("theirs")).toInteger()),
		           { T(m_backend, "bar.conflict.mine"), T(m_backend, "bar.conflict.theirs"), T(m_backend, "bar.conflict.both") }, [this](int i) {
			           const char* choices[] = { "mine", "theirs", "both" };
			           if (i >= 0)
				           saveAll(QString::fromLatin1(choices[i]));
		           });
		return;
	}
	showEditResult(r, r.value(QStringLiteral("notice")).toString(T(m_backend, "status.saved")));
	reloadViews();
}

namespace rjmw {

QString text(RjBackend* b, const char* key)
{
	return T(b, key);
}

QJsonObject parseJson(const QString& s)
{
	return parseObject(s);
}

QIcon icon(const QString& kind, const QString& light, bool folder, int session)
{
	return protocolIcon(kind, light, folder, session);
}

} // namespace rjmw

void MainWindow::addFavoritesFolder()
{
	if (!m_showFavorites || !m_backend->isVaultOpen())
		return;
	const QJsonArray favs = QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("favorites_tree") } }).toUtf8()).array();
	if (favs.isEmpty())
		return;
	const QColor star = m_starColor;
	auto* folder = new QStandardItem(faIcon(QStringLiteral("r-star"), star, star), T(m_backend, "view.favorites").remove(QStringLiteral("★ ")));
	folder->setData(true, RoleFavFolder);
	folder->setData(true, RoleFolder);
	folder->setEditable(false);
	folder->setDragEnabled(false);
	folder->setDropEnabled(false);
	QFont f = folder->font();
	f.setWeight(QFont::DemiBold);
	folder->setFont(f);
	addNodes(folder, favs);
	// Verweise: kein Ziehen, keine Unterknoten (Gäste eines Hypervisors lädt nur der Eintrag im Baum).
	for (int r = 0; r < folder->rowCount(); ++r)
	{
		QStandardItem* it = folder->child(r);
		it->setData(true, RoleFavLink);
		it->setDragEnabled(false);
		it->setDropEnabled(false);
		if (it->rowCount() > 0)
			it->removeRows(0, it->rowCount());
	}
	m_model->invisibleRootItem()->insertRow(0, folder);
	if (m_favOpen)
	{
		const bool was = m_autoExpand;
		m_autoExpand = true;
		m_tree->expand(folder->index());
		m_autoExpand = was;
	}
}

void MainWindow::applyTreePrefs()
{
	const QJsonObject a = parseObject(m_backend->uiPrefsJson());
	m_flyOpenMs = a.value(QStringLiteral("flyout_open_ms")).toInt(300);
	m_flyCloseMs = a.value(QStringLiteral("flyout_close_ms")).toInt(500);
	const bool favs = a.value(QStringLiteral("show_favorites")).toBool(true);
	if (favs != m_showFavorites)
	{
		m_showFavorites = favs;
		if (m_model)
			reloadTree();
	}
	setPinned(a.value(QStringLiteral("tree_pinned")).toBool(true), false);
	updatePinButton();
}

void MainWindow::updatePinButton()
{
	if (!m_pinButton)
		return;
	const QColor accent = palette().color(QPalette::Highlight);
	m_pinButton->setIcon(m_pinned ? faIcon(QStringLiteral("thumbtack"), accent, accent) : faIcon(QStringLiteral("thumbtack-slash")));
	const QString tip = T(m_backend, m_pinned ? "tree.pin.on" : "tree.pin.off");
	m_pinButton->setToolTip(tip);
	m_pinButton->setAccessibleName(tip);
}

void MainWindow::setPinned(bool on, bool save)
{
	if (save)
		m_backend->setAppearance(QString::fromUtf8(QJsonDocument(QJsonObject{ { QStringLiteral("tree_pinned"), on } }).toJson(QJsonDocument::Compact)));
	if (on == m_pinned)
	{
		updatePinButton();
		return;
	}
	m_flyOpenTimer.stop();
	m_flyCloseTimer.stop();
	if (on)
	{
		// Zurück in den Teiler, ausgeklappt.
		m_flyOpen = false;
		m_pinned = true;
		m_rail->hide();
		m_leftPane->setAutoFillBackground(false);
		m_leftPane->setStyleSheet(QString());
		m_leftPane->setAttribute(Qt::WA_StyledBackground, false);
		m_split->insertWidget(0, m_leftPane);
		m_treeHidden = false;
		m_leftPane->show();
		m_split->setSizes({ m_treeWidth, qMax(200, m_split->width() - m_treeWidth) });
	}
	else
	{
		if (!m_treeHidden && !m_split->sizes().isEmpty() && m_split->sizes().first() >= 100)
			m_treeWidth = m_split->sizes().first();
		m_pinned = false;
		m_flyOpen = false;
		m_treeHidden = true;
		m_leftPane->hide();
		m_leftPane->setParent(m_center);
		m_leftPane->setAutoFillBackground(true);
		m_leftPane->setObjectName(QStringLiteral("treeFlyout"));
		m_leftPane->setAttribute(Qt::WA_StyledBackground, true);
		m_leftPane->setStyleSheet(QStringLiteral("QWidget#treeFlyout { background: palette(window); border: 1px solid palette(mid); border-left: none; "
		                                         "border-top-right-radius: 8px; border-bottom-right-radius: 8px; }"));
		m_leftPane->installEventFilter(this);
		m_leftPane->hide();
		m_rail->show();
	}
	updatePinButton();
	placeBrand();
	if (m_treeButton)
		m_treeButton->setText(m_pinned && !m_treeHidden ? QStringLiteral("\u00ab") : QStringLiteral("\u00bb"));
}

void MainWindow::placeFlyout()
{
	if (m_pinned || !m_flyOpen)
		return;
	// Unter der Reiterzeile, damit Tabs und Logo sichtbar bleiben.
	const int x = m_rail->isVisible() ? m_rail->geometry().right() + 1 : 0;
	const int w = qBound(220, m_treeWidth, qMax(220, m_center->width() - x - 120));
	QTabBar* bar = m_tabs->tabBar();
	const int y = bar->isVisible() ? qMax(0, bar->mapTo(m_center, QPoint(0, bar->height())).y()) : 0;
	m_leftPane->setGeometry(x, y, w, m_center->height() - y);
}

void MainWindow::openFlyout()
{
	m_flyOpenTimer.stop();
	m_flyCloseTimer.stop();
	if (m_pinned || m_flyOpen)
		return;
	m_flyOpen = true;
	m_treeHidden = false;
	placeFlyout();
	m_leftPane->show();
	m_leftPane->raise();
	m_leftInner->setVisible(true);
}

void MainWindow::closeFlyout()
{
	m_flyOpenTimer.stop();
	m_flyCloseTimer.stop();
	if (m_pinned || !m_flyOpen)
		return;
	const bool hadFocus = m_leftPane->isAncestorOf(QApplication::focusWidget());
	m_flyOpen = false;
	m_treeHidden = true;
	m_leftPane->hide();
	if (hadFocus && m_tabs->currentWidget())
		m_tabs->currentWidget()->setFocus();
}

void MainWindow::flyoutForScreenshot(bool unpinned, bool open)
{
	setPinned(!unpinned, false);
	if (unpinned && open)
		openFlyout();
}

void MainWindow::openNews()
{
	for (int i = 0; i < m_tabs->count(); ++i)
		if (m_tabs->widget(i)->objectName() == QLatin1String("newsTab"))
		{
			m_tabs->setCurrentIndex(i);
			return;
		}
	auto* view = new QTextBrowser(m_tabs);
	view->setObjectName(QStringLiteral("newsTab"));
	view->setOpenExternalLinks(true);
	view->setFrameShape(QFrame::NoFrame);
	view->document()->setDocumentMargin(24);
	// Kopf mittig (Logo, Titel, Link auf GitHub), darunter die Releasenotes aus dem Markdown.
	{
		QTextDocument md;
		md.setMarkdown(m_backend->legalText(QStringLiteral("releasenotes")));
		QString body = md.toHtml();
		const int open = body.indexOf(QStringLiteral("<body"));
		const int start = open >= 0 ? body.indexOf(QLatin1Char('>'), open) + 1 : 0;
		const int end = body.lastIndexOf(QStringLiteral("</body>"));
		body = body.mid(start, end > start ? end - start : -1);
		const QString head = QStringLiteral("<p align=\"center\"><img src=\":/logo/remotejoe-128.png\" width=\"96\" height=\"96\"></p>"
		                                    "<p align=\"center\" style=\"font-size: x-large; font-weight: 600;\">%1</p>"
		                                    "<p align=\"center\"><a href=\"https://github.com/joeMJ/remotejoe\">github.com/joeMJ/remotejoe</a></p><hr>")
		                       .arg(T(m_backend, "news.heading").toHtmlEscaped());
		view->setHtml(head + body);
	}
	const int i = m_tabs->addTab(view, faIcon(QStringLiteral("r-lightbulb")), T(m_backend, "news.title"));
	m_tabs->setCurrentIndex(i);
}

void MainWindow::showNewsIfNew()
{
	const QString version = m_backend->appVersion();
	const QString seen = parseObject(m_backend->uiPrefsJson()).value(QStringLiteral("notes_seen")).toString();
	if (seen == version)
		return;
	// Bisher benutzt (gespeicherter Fensterzustand): Update, Neuigkeiten zeigen. Sonst neue Installation.
	const bool used = parseObject(m_backend->windowStateJson()).value(QStringLiteral("saved")).toBool();
	m_backend->setAppearance(QString::fromUtf8(QJsonDocument(QJsonObject{ { QStringLiteral("notes_seen"), version } }).toJson(QJsonDocument::Compact)));
	if (used)
		openNews();
}

QWidget* MainWindow::showFirstHints(bool force)
{
	if (!force)
	{
		if (parseObject(m_backend->uiPrefsJson()).value(QStringLiteral("first_hints_off")).toBool())
			return nullptr;
		// Erst mit angelegter bzw. geöffneter Schlüsseldatenbank; sobald es ein Ziel gibt, entfallen sie von selbst.
		if (!m_backend->isVaultOpen() || !(m_emptyHint && m_emptyHint->isVisible()))
			return nullptr;
	}
	if (auto* old = findChild<QWidget*>(QStringLiteral("coachOverlay")))
		return old;
	QList<CoachOverlay::Hint> hints;
	if (auto* burger = m_titleBar->controls()->findChild<QToolButton*>(QString(), Qt::FindDirectChildrenOnly); burger)
	{
		// Der Hamburger-Knopf ist der Knopf mit Menü.
		for (auto* b : m_titleBar->controls()->findChildren<QToolButton*>(QString(), Qt::FindDirectChildrenOnly))
			if (b->menu())
				hints.append({ b, T(m_backend, "hint.menu") });
	}
	// Anlegen geht über das Kontextmenü im Baum: Pfeil auf den Zielbereich („Rechtsklick hier“).
	QWidget* add = (m_emptyButton && m_emptyButton->isVisible()) ? static_cast<QWidget*>(m_emptyButton) : static_cast<QWidget*>(m_tree->viewport());
	if (m_pinned && m_leftPane->isVisible())
		hints.append({ add, T(m_backend, m_backend->isVaultOpen() ? "hint.add" : "hint.add.vault") });
	else
		hints.append({ m_rail, T(m_backend, "hint.add.rail") });
	auto* o = new CoachOverlay(this, hints, T(m_backend, "hint.dontshow"), T(m_backend, "hint.ok"), [this](bool off) {
		if (off)
			m_backend->setAppearance(QString::fromUtf8(QJsonDocument(QJsonObject{ { QStringLiteral("first_hints_off"), true } }).toJson(QJsonDocument::Compact)));
	});
	o->setObjectName(QStringLiteral("coachOverlay"));
	return o;
}
