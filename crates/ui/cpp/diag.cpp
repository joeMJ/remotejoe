#include "diag.h"

#include <QtCore/QAbstractNativeEventFilter>
#include <QtCore/QDateTime>
#include <QtCore/QDir>
#include <QtCore/QFile>
#include <QtCore/QFileInfo>
#include <QtCore/QElapsedTimer>
#include <QtCore/QHash>
#include <QtCore/QMutex>
#include <QtCore/QPointer>
#include <QtCore/QSysInfo>
#include <QtCore/QTextStream>
#include <QtGui/QGuiApplication>
#include <QtGui/QKeyEvent>
#include <QtGui/QMouseEvent>
#include <QtGui/QFocusEvent>
#include <QtGui/QScreen>
#include <QtGui/QShortcut>
#include <QtGui/QWindow>
#include <QtWidgets/QAbstractItemView>
#include <QtWidgets/QApplication>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QWidget>

#include <deque>

#ifdef Q_OS_WIN
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#endif

namespace {

constexpr size_t RingMax = 6000;
constexpr int KeepIncidents = 20;

QMutex g_mutex;
std::deque<QString> g_ring;
QFile* g_live = nullptr;
bool g_started = false;
QElapsedTimer g_lastAuto;

QString diagDir()
{
	const QString d = QDir(QDir::tempPath()).filePath(QStringLiteral("remotejoe-diag"));
	QDir().mkpath(d);
	return d;
}

QString stamp()
{
	return QDateTime::currentDateTime().toString(QStringLiteral("HH:mm:ss.zzz"));
}

QString objName(const QObject* o)
{
	if (!o)
		return QStringLiteral("-");
	QString s = QString::fromLatin1(o->metaObject()->className());
	if (!o->objectName().isEmpty())
		s += QStringLiteral(" '%1'").arg(o->objectName());
	return s;
}

/// Widget zu einem Qt-Fenster (für lesbare Namen).
QWidget* widgetOf(const QWindow* w)
{
	if (!w)
		return nullptr;
	for (QWidget* tl : QApplication::topLevelWidgets())
		if (tl->windowHandle() == w)
			return tl;
	return nullptr;
}

QString windowName(const QWindow* w)
{
	if (!w)
		return QStringLiteral("-");
	if (QWidget* tl = widgetOf(w))
		return objName(tl);
	return objName(w);
}

QString mods(Qt::KeyboardModifiers m)
{
	QStringList l;
	if (m & Qt::ShiftModifier)
		l << QStringLiteral("Umschalt");
	if (m & Qt::ControlModifier)
		l << QStringLiteral("Strg");
	if (m & Qt::AltModifier)
		l << QStringLiteral("Alt");
	if (m & Qt::MetaModifier)
		l << QStringLiteral("Win");
	return l.isEmpty() ? QStringLiteral("-") : l.join(QLatin1Char('+'));
}

QString keyName(int key)
{
	switch (key)
	{
	case Qt::Key_Shift:
		return QStringLiteral("Umschalt");
	case Qt::Key_Control:
		return QStringLiteral("Strg");
	case Qt::Key_Alt:
		return QStringLiteral("Alt");
	case Qt::Key_AltGr:
		return QStringLiteral("AltGr");
	case Qt::Key_Meta:
		return QStringLiteral("Win");
	case Qt::Key_CapsLock:
		return QStringLiteral("Feststell");
	case Qt::Key_Escape:
		return QStringLiteral("Esc");
	case Qt::Key_Tab:
		return QStringLiteral("Tab");
	case Qt::Key_Return:
	case Qt::Key_Enter:
		return QStringLiteral("Eingabe");
	case Qt::Key_Up:
	case Qt::Key_Down:
	case Qt::Key_F4:
		return QStringLiteral("Listentaste");
	default:
		// Zeichentasten werden nicht aufgezeichnet.
		return QStringLiteral("andere");
	}
}

#ifdef Q_OS_WIN

QHash<DWORD, QString> g_exe;

QString exeOf(DWORD pid)
{
	if (!pid)
		return QStringLiteral("-");
	auto it = g_exe.constFind(pid);
	if (it != g_exe.constEnd())
		return *it;
	QString name = QStringLiteral("?");
	if (HANDLE h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid))
	{
		wchar_t buf[MAX_PATH] = {};
		DWORD n = MAX_PATH;
		if (QueryFullProcessImageNameW(h, 0, buf, &n))
			name = QFileInfo(QString::fromWCharArray(buf, int(n))).fileName();
		CloseHandle(h);
	}
	if (g_exe.size() > 2000)
		g_exe.clear();
	g_exe.insert(pid, name);
	return name;
}

QString hwndHex(HWND h)
{
	return QStringLiteral("0x%1").arg(quintptr(h), 0, 16);
}

/// Fenster: Kennung, Klasse, Programm, Prozess; Titel nur bei eigenen Fenstern.
QString hwndInfo(HWND h)
{
	if (!h)
		return QStringLiteral("0 (kein Fenster)");
	if (!IsWindow(h))
		return hwndHex(h) + QStringLiteral(" (ungültig)");
	wchar_t cls[128] = {};
	GetClassNameW(h, cls, 128);
	DWORD pid = 0;
	GetWindowThreadProcessId(h, &pid);
	QString s = QStringLiteral("%1 %2 [%3 pid %4]").arg(hwndHex(h), QString::fromWCharArray(cls), exeOf(pid)).arg(pid);
	if (!IsWindowVisible(h))
		s += QStringLiteral(" unsichtbar");
	if (pid == GetCurrentProcessId())
	{
		wchar_t title[96] = {};
		GetWindowTextW(h, title, 96);
		s += QStringLiteral(" \"%1\"").arg(QString::fromWCharArray(title));
		for (QWindow* w : QGuiApplication::allWindows())
			if (w->handle() && HWND(w->winId()) == h)
			{
				s += QStringLiteral(" Qt=") + windowName(w);
				break;
			}
	}
	return s;
}

QString threadExe(DWORD tid)
{
	DWORD pid = 0;
	if (HANDLE t = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, FALSE, tid))
	{
		pid = GetProcessIdOfThread(t);
		CloseHandle(t);
	}
	return QStringLiteral("Thread %1 [%2 pid %3]").arg(tid).arg(exeOf(pid)).arg(pid);
}

QString vkName(WPARAM vk)
{
	switch (vk)
	{
	case VK_SHIFT:
		return QStringLiteral("Umschalt");
	case VK_LSHIFT:
		return QStringLiteral("Umschalt-links");
	case VK_RSHIFT:
		return QStringLiteral("Umschalt-rechts");
	case VK_CONTROL:
	case VK_LCONTROL:
	case VK_RCONTROL:
		return QStringLiteral("Strg");
	case VK_MENU:
	case VK_LMENU:
	case VK_RMENU:
		return QStringLiteral("Alt");
	case VK_LWIN:
	case VK_RWIN:
		return QStringLiteral("Win");
	case VK_CAPITAL:
		return QStringLiteral("Feststell");
	case VK_ESCAPE:
		return QStringLiteral("Esc");
	case VK_TAB:
		return QStringLiteral("Tab");
	default:
		return QString();
	}
}

QString keyStates()
{
	auto st = [](int vk) {
		const SHORT a = GetAsyncKeyState(vk);
		const SHORT k = GetKeyState(vk);
		return QStringLiteral("%1/%2").arg((a & 0x8000) ? QStringLiteral("ab") : QStringLiteral("auf"), (k & 0x8000) ? QStringLiteral("ab") : QStringLiteral("auf"));
	};
	return QStringLiteral("Tasten (System/Thread): Umschalt %1, Umschalt-l %2, Umschalt-r %3, Strg %4, Alt %5, Win %6, Maus-links %7, Feststell %8")
	    .arg(st(VK_SHIFT), st(VK_LSHIFT), st(VK_RSHIFT), st(VK_CONTROL), st(VK_MENU), st(VK_LWIN), st(VK_LBUTTON))
	    .arg((GetKeyState(VK_CAPITAL) & 1) ? QStringLiteral("an") : QStringLiteral("aus"));
}

QString guiThreadInfo(DWORD tid, const QString& label)
{
	GUITHREADINFO g{};
	g.cbSize = sizeof(g);
	if (!GetGUIThreadInfo(tid, &g))
		return label + QStringLiteral(": nicht lesbar (Fehler %1)").arg(GetLastError());
	QStringList f;
	if (g.flags & GUI_CARETBLINKING)
		f << QStringLiteral("Schreibmarke");
	if (g.flags & GUI_INMENUMODE)
		f << QStringLiteral("Menümodus");
	if (g.flags & GUI_INMOVESIZE)
		f << QStringLiteral("Verschieben/Größe");
	if (g.flags & GUI_POPUPMENUMODE)
		f << QStringLiteral("Popupmenü");
	if (g.flags & GUI_SYSTEMMENUMODE)
		f << QStringLiteral("Systemmenü");
	return QStringLiteral("%1: Flags %2\n    aktiv   %3\n    Fokus   %4\n    Maus    %5\n    Menü    %6\n    Bewegen %7")
	    .arg(label, f.isEmpty() ? QStringLiteral("-") : f.join(QLatin1Char(',')), hwndInfo(g.hwndActive), hwndInfo(g.hwndFocus), hwndInfo(g.hwndCapture),
	         hwndInfo(g.hwndMenuOwner), hwndInfo(g.hwndMoveSize));
}

struct EnumCtx
{
	QStringList* out;
	int count;
};

BOOL CALLBACK enumForeignChild(HWND h, LPARAM p)
{
	auto* c = reinterpret_cast<EnumCtx*>(p);
	DWORD pid = 0;
	GetWindowThreadProcessId(h, &pid);
	if (pid != GetCurrentProcessId())
		c->out->append(QStringLiteral("      fremdes Kindfenster: ") + hwndInfo(h));
	else
		++c->count;
	return TRUE;
}

BOOL CALLBACK enumOwnTop(HWND h, LPARAM p)
{
	auto* out = reinterpret_cast<QStringList*>(p);
	DWORD pid = 0;
	GetWindowThreadProcessId(h, &pid);
	if (pid != GetCurrentProcessId())
		return TRUE;
	RECT r{};
	GetWindowRect(h, &r);
	const LONG_PTR ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
	out->append(QStringLiteral("  %1 Rahmen %2,%3 %4x%5%6").arg(hwndInfo(h)).arg(r.left).arg(r.top).arg(r.right - r.left).arg(r.bottom - r.top)
	                .arg((ex & WS_EX_TOPMOST) ? QStringLiteral(" zuoberst") : QString()));
	if (IsWindowVisible(h))
	{
		EnumCtx c{ out, 0 };
		EnumChildWindows(h, enumForeignChild, reinterpret_cast<LPARAM>(&c));
		out->append(QStringLiteral("      eigene Kindfenster: %1").arg(c.count));
	}
	return TRUE;
}

struct ZCtx
{
	QStringList* out;
	int n;
};

BOOL CALLBACK enumZOrder(HWND h, LPARAM p)
{
	auto* c = reinterpret_cast<ZCtx*>(p);
	if (!IsWindowVisible(h))
		return TRUE;
	RECT r{};
	GetWindowRect(h, &r);
	if (r.right - r.left <= 0 || r.bottom - r.top <= 0)
		return TRUE;
	const LONG_PTR ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
	c->out->append(QStringLiteral("  %1. %2 Rahmen %3,%4 %5x%6%7").arg(++c->n).arg(hwndInfo(h)).arg(r.left).arg(r.top).arg(r.right - r.left).arg(r.bottom - r.top)
	                   .arg((ex & WS_EX_TOPMOST) ? QStringLiteral(" zuoberst") : QString()));
	return c->n < 30;
}

QString winEventName(DWORD e)
{
	switch (e)
	{
	case EVENT_SYSTEM_FOREGROUND:
		return QStringLiteral("Vordergrund");
	case EVENT_SYSTEM_MENUSTART:
		return QStringLiteral("Menü-Beginn");
	case EVENT_SYSTEM_MENUEND:
		return QStringLiteral("Menü-Ende");
	case EVENT_SYSTEM_MENUPOPUPSTART:
		return QStringLiteral("Popupmenü-Beginn");
	case EVENT_SYSTEM_MENUPOPUPEND:
		return QStringLiteral("Popupmenü-Ende");
	case EVENT_SYSTEM_CAPTURESTART:
		return QStringLiteral("Mausfang-Beginn");
	case EVENT_SYSTEM_CAPTUREEND:
		return QStringLiteral("Mausfang-Ende");
	case EVENT_SYSTEM_MOVESIZESTART:
		return QStringLiteral("Bewegen-Beginn");
	case EVENT_SYSTEM_MOVESIZEEND:
		return QStringLiteral("Bewegen-Ende");
	case EVENT_SYSTEM_DRAGDROPSTART:
		return QStringLiteral("Ziehen-Beginn");
	case EVENT_SYSTEM_DRAGDROPEND:
		return QStringLiteral("Ziehen-Ende");
	case EVENT_SYSTEM_DIALOGSTART:
		return QStringLiteral("Dialog-Beginn");
	case EVENT_SYSTEM_DIALOGEND:
		return QStringLiteral("Dialog-Ende");
	case EVENT_SYSTEM_SWITCHSTART:
		return QStringLiteral("Alt-Tab-Beginn");
	case EVENT_SYSTEM_SWITCHEND:
		return QStringLiteral("Alt-Tab-Ende");
	case EVENT_SYSTEM_MINIMIZESTART:
		return QStringLiteral("Minimieren-Beginn");
	case EVENT_SYSTEM_MINIMIZEEND:
		return QStringLiteral("Minimieren-Ende");
	case EVENT_SYSTEM_DESKTOPSWITCH:
		return QStringLiteral("Desktopwechsel");
	case EVENT_OBJECT_SHOW:
		return QStringLiteral("Fenster-gezeigt");
	case EVENT_OBJECT_HIDE:
		return QStringLiteral("Fenster-versteckt");
	case EVENT_OBJECT_FOCUS:
		return QStringLiteral("Fokus");
	default:
		return QStringLiteral("Ereignis 0x%1").arg(e, 0, 16);
	}
}

void CALLBACK winEventHook(HWINEVENTHOOK, DWORD event, HWND hwnd, LONG idObject, LONG idChild, DWORD thread, DWORD)
{
	if (event == EVENT_OBJECT_SHOW || event == EVENT_OBJECT_HIDE)
	{
		// Nur ganze Fenster der obersten Ebene (keine Bedienelemente, keine Schreibmarke).
		if (idObject != OBJID_WINDOW || idChild != CHILDID_SELF || !hwnd || GetAncestor(hwnd, GA_ROOT) != hwnd)
			return;
	}
	else if (event == EVENT_OBJECT_FOCUS)
	{
		// Nur Fokus in eigenen Fenstern oder in fremden Fenstern, die in eigenen stecken (Web-Tab).
		HWND root = hwnd ? GetAncestor(hwnd, GA_ROOT) : nullptr;
		DWORD pid = 0;
		if (root)
			GetWindowThreadProcessId(root, &pid);
		if (pid != GetCurrentProcessId())
			return;
	}
	else if (event == EVENT_SYSTEM_SCROLLINGSTART || event == EVENT_SYSTEM_SCROLLINGEND || event == EVENT_SYSTEM_CONTEXTHELPSTART || event == EVENT_SYSTEM_CONTEXTHELPEND)
		return;
	rjdiag::log(QStringLiteral("[win] %1 %2 (%3)").arg(winEventName(event), hwndInfo(hwnd), threadExe(thread)));
}

QString msgName(UINT m)
{
	switch (m)
	{
	case WM_ACTIVATE:
		return QStringLiteral("WM_ACTIVATE");
	case WM_ACTIVATEAPP:
		return QStringLiteral("WM_ACTIVATEAPP");
	case WM_NCACTIVATE:
		return QStringLiteral("WM_NCACTIVATE");
	case WM_SETFOCUS:
		return QStringLiteral("WM_SETFOCUS");
	case WM_KILLFOCUS:
		return QStringLiteral("WM_KILLFOCUS");
	case WM_CAPTURECHANGED:
		return QStringLiteral("WM_CAPTURECHANGED");
	case WM_MOUSEACTIVATE:
		return QStringLiteral("WM_MOUSEACTIVATE");
	case WM_CANCELMODE:
		return QStringLiteral("WM_CANCELMODE");
	case WM_SHOWWINDOW:
		return QStringLiteral("WM_SHOWWINDOW");
	case WM_WINDOWPOSCHANGED:
		return QStringLiteral("WM_WINDOWPOSCHANGED");
	case WM_ENTERMENULOOP:
		return QStringLiteral("WM_ENTERMENULOOP");
	case WM_EXITMENULOOP:
		return QStringLiteral("WM_EXITMENULOOP");
	case WM_ENTERSIZEMOVE:
		return QStringLiteral("WM_ENTERSIZEMOVE");
	case WM_EXITSIZEMOVE:
		return QStringLiteral("WM_EXITSIZEMOVE");
	case WM_SYSCOMMAND:
		return QStringLiteral("WM_SYSCOMMAND");
	case WM_LBUTTONDOWN:
		return QStringLiteral("WM_LBUTTONDOWN");
	case WM_LBUTTONUP:
		return QStringLiteral("WM_LBUTTONUP");
	case WM_LBUTTONDBLCLK:
		return QStringLiteral("WM_LBUTTONDBLCLK");
	case WM_RBUTTONDOWN:
		return QStringLiteral("WM_RBUTTONDOWN");
	case WM_RBUTTONUP:
		return QStringLiteral("WM_RBUTTONUP");
	case WM_NCLBUTTONDOWN:
		return QStringLiteral("WM_NCLBUTTONDOWN");
	case WM_NCLBUTTONUP:
		return QStringLiteral("WM_NCLBUTTONUP");
	case WM_KEYDOWN:
		return QStringLiteral("WM_KEYDOWN");
	case WM_KEYUP:
		return QStringLiteral("WM_KEYUP");
	case WM_SYSKEYDOWN:
		return QStringLiteral("WM_SYSKEYDOWN");
	case WM_SYSKEYUP:
		return QStringLiteral("WM_SYSKEYUP");
	case WM_INPUTLANGCHANGE:
		return QStringLiteral("WM_INPUTLANGCHANGE");
	case WM_DISPLAYCHANGE:
		return QStringLiteral("WM_DISPLAYCHANGE");
	case WM_DPICHANGED:
		return QStringLiteral("WM_DPICHANGED");
	case WM_SETTINGCHANGE:
		return QStringLiteral("WM_SETTINGCHANGE");
	case WM_POWERBROADCAST:
		return QStringLiteral("WM_POWERBROADCAST");
	case WM_WTSSESSION_CHANGE:
		return QStringLiteral("WM_WTSSESSION_CHANGE");
	default:
		return QString();
	}
}

QString mkFlags(WPARAM w)
{
	QStringList l;
	if (w & MK_SHIFT)
		l << QStringLiteral("Umschalt");
	if (w & MK_CONTROL)
		l << QStringLiteral("Strg");
	if (w & MK_LBUTTON)
		l << QStringLiteral("links");
	if (w & MK_RBUTTON)
		l << QStringLiteral("rechts");
	return l.isEmpty() ? QStringLiteral("-") : l.join(QLatin1Char('+'));
}

/// Fensternachrichten an eigene Fenster (gesendet und eingereiht), soweit sie Aktivierung, Fokus, Mausfang,
/// Sichtbarkeit und Eingabe betreffen.
class NativeFilter : public QAbstractNativeEventFilter
{
public:
	bool nativeEventFilter(const QByteArray& type, void* message, qintptr*) override
	{
		if (type != "windows_generic_MSG")
			return false;
		const MSG* m = static_cast<const MSG*>(message);
		const QString name = msgName(m->message);
		if (name.isEmpty())
			return false;
		QString detail;
		switch (m->message)
		{
		case WM_ACTIVATE:
			detail = QStringLiteral("Zustand %1%2, anderes Fenster %3")
			             .arg(LOWORD(m->wParam) == WA_INACTIVE ? QStringLiteral("inaktiv") : LOWORD(m->wParam) == WA_CLICKACTIVE ? QStringLiteral("aktiv-durch-Klick") : QStringLiteral("aktiv"))
			             .arg(HIWORD(m->wParam) ? QStringLiteral(" minimiert") : QString(), hwndInfo(HWND(m->lParam)));
			break;
		case WM_ACTIVATEAPP:
			detail = QStringLiteral("%1, andere Anwendung %2").arg(m->wParam ? QStringLiteral("aktiv") : QStringLiteral("inaktiv"), threadExe(DWORD(m->lParam)));
			break;
		case WM_NCACTIVATE:
			detail = m->wParam ? QStringLiteral("aktiv") : QStringLiteral("inaktiv");
			break;
		case WM_SETFOCUS:
			detail = QStringLiteral("vorher %1").arg(hwndInfo(HWND(m->wParam)));
			break;
		case WM_KILLFOCUS:
			detail = QStringLiteral("nachher %1").arg(hwndInfo(HWND(m->wParam)));
			break;
		case WM_CAPTURECHANGED:
			detail = QStringLiteral("neuer Mausfang %1").arg(hwndInfo(HWND(m->lParam)));
			break;
		case WM_MOUSEACTIVATE:
			detail = QStringLiteral("oberstes %1, Treffer %2, Nachricht 0x%3").arg(hwndInfo(HWND(m->wParam))).arg(short(LOWORD(m->lParam))).arg(HIWORD(m->lParam), 0, 16);
			break;
		case WM_SHOWWINDOW:
			detail = QStringLiteral("%1, Grund %2").arg(m->wParam ? QStringLiteral("zeigen") : QStringLiteral("verstecken")).arg(m->lParam);
			break;
		case WM_WINDOWPOSCHANGED:
		{
			const auto* p = reinterpret_cast<const WINDOWPOS*>(m->lParam);
			const UINT f = p ? p->flags : 0;
			// Nur Sichtbarkeit, Aktivierung und Stapelreihenfolge; reine Größen-/Lageänderungen nicht.
			if (!(f & (SWP_SHOWWINDOW | SWP_HIDEWINDOW)) && (f & SWP_NOZORDER) && (f & SWP_NOACTIVATE))
				return false;
			detail = QStringLiteral("Flags 0x%1%2%3%4, nach %5").arg(f, 0, 16)
			             .arg((f & SWP_SHOWWINDOW) ? QStringLiteral(" zeigen") : QString(), (f & SWP_HIDEWINDOW) ? QStringLiteral(" verstecken") : QString(),
			                  (f & SWP_NOACTIVATE) ? QString() : QStringLiteral(" aktivieren"))
			             .arg(p ? hwndInfo(p->hwndInsertAfter) : QStringLiteral("-"));
			break;
		}
		case WM_SYSCOMMAND:
			detail = QStringLiteral("Befehl 0x%1").arg(m->wParam & 0xFFF0, 0, 16);
			break;
		case WM_LBUTTONDOWN:
		case WM_LBUTTONUP:
		case WM_LBUTTONDBLCLK:
		case WM_RBUTTONDOWN:
		case WM_RBUTTONUP:
			detail = QStringLiteral("bei %1,%2, Tasten %3").arg(short(LOWORD(m->lParam))).arg(short(HIWORD(m->lParam))).arg(mkFlags(m->wParam));
			break;
		case WM_NCLBUTTONDOWN:
		case WM_NCLBUTTONUP:
			detail = QStringLiteral("Treffer %1").arg(m->wParam);
			break;
		case WM_KEYDOWN:
		case WM_KEYUP:
		case WM_SYSKEYDOWN:
		case WM_SYSKEYUP:
		{
			const QString k = vkName(m->wParam);
			detail = QStringLiteral("%1, Scancode %2%3, vorher %4")
			             .arg(k.isEmpty() ? QStringLiteral("andere Taste") : k)
			             .arg((m->lParam >> 16) & 0xFF)
			             .arg(((m->lParam >> 24) & 1) ? QStringLiteral(" erweitert") : QString(),
			                  ((m->lParam >> 30) & 1) ? QStringLiteral("ab") : QStringLiteral("auf"));
			break;
		}
		default:
			detail = QStringLiteral("w 0x%1 l 0x%2").arg(quintptr(m->wParam), 0, 16).arg(quintptr(m->lParam), 0, 16);
			break;
		}
		rjdiag::log(QStringLiteral("[msg] %1 an %2: %3").arg(name, hwndInfo(m->hwnd), detail));
		return false;
	}
};

#endif // Q_OS_WIN

/// Qt-Ereignisse: Maus und Tasten an Fenstern, Fokus, Aktivierung, Zeigen/Verstecken eigenständiger Fenster
/// (Auswahllisten, Menüs, Dialoge).
class QtFilter : public QObject
{
public:
	using QObject::QObject;

protected:
	bool eventFilter(QObject* o, QEvent* e) override
	{
		switch (e->type())
		{
		case QEvent::MouseButtonPress:
		case QEvent::MouseButtonRelease:
		case QEvent::MouseButtonDblClick:
			if (o->isWindowType())
			{
				const auto* me = static_cast<QMouseEvent*>(e);
				const QPoint g = me->globalPosition().toPoint();
				const char* what = e->type() == QEvent::MouseButtonPress ? "ab" : e->type() == QEvent::MouseButtonRelease ? "auf" : "doppelt";
				QWidget* under = QApplication::widgetAt(g);
				rjdiag::log(QStringLiteral("[qt] Maus %1 Taste %2 bei %3,%4 an %5, unter Zeiger %6, Qt-Umschalttasten %7")
				                .arg(QLatin1String(what)).arg(int(me->button())).arg(g.x()).arg(g.y())
				                .arg(windowName(static_cast<QWindow*>(o)), objName(under), mods(me->modifiers())));
#ifdef Q_OS_WIN
				if (e->type() == QEvent::MouseButtonPress)
				{
					const bool qtShift = me->modifiers() & Qt::ShiftModifier;
					const bool winShift = GetAsyncKeyState(VK_SHIFT) & 0x8000;
					if (qtShift != winShift)
					{
						rjdiag::log(QStringLiteral("[qt] Umschalttaste uneinig: Qt %1, Windows %2").arg(qtShift ? QStringLiteral("ab") : QStringLiteral("auf"), winShift ? QStringLiteral("ab") : QStringLiteral("auf")));
						if (!g_lastAuto.isValid() || g_lastAuto.elapsed() > 60000)
						{
							g_lastAuto.start();
							rjdiag::incident(QStringLiteral("Umschalttaste uneinig (Qt %1, Windows %2)").arg(qtShift ? QStringLiteral("gedrückt") : QStringLiteral("los"), winShift ? QStringLiteral("gedrückt") : QStringLiteral("los")));
						}
					}
				}
#endif
			}
			break;
		case QEvent::KeyPress:
		case QEvent::KeyRelease:
			if (o->isWindowType())
			{
				const auto* ke = static_cast<QKeyEvent*>(e);
				rjdiag::log(QStringLiteral("[qt] Taste %1 %2%3 an %4, Qt-Umschalttasten %5")
				                .arg(e->type() == QEvent::KeyPress ? QStringLiteral("ab") : QStringLiteral("auf"), keyName(ke->key()),
				                     ke->isAutoRepeat() ? QStringLiteral(" (Wiederholung)") : QString(), windowName(static_cast<QWindow*>(o)), mods(ke->modifiers())));
			}
			break;
		case QEvent::FocusIn:
		case QEvent::FocusOut:
			if (o->isWindowType())
				rjdiag::log(QStringLiteral("[qt] Fenster-Fokus %1 %2, Grund %3")
				                .arg(e->type() == QEvent::FocusIn ? QStringLiteral("rein") : QStringLiteral("raus"), windowName(static_cast<QWindow*>(o)))
				                .arg(int(static_cast<QFocusEvent*>(e)->reason())));
			break;
		case QEvent::Show:
		case QEvent::Hide:
		case QEvent::WindowActivate:
		case QEvent::WindowDeactivate:
			if (o->isWidgetType() && static_cast<QWidget*>(o)->isWindow())
			{
				const auto* w = static_cast<QWidget*>(o);
				const char* what = e->type() == QEvent::Show ? "gezeigt" : e->type() == QEvent::Hide ? "versteckt" : e->type() == QEvent::WindowActivate ? "aktiviert" : "deaktiviert";
				const QRect r = w->frameGeometry();
				rjdiag::log(QStringLiteral("[qt] Fenster %1 %2, Typ 0x%3, Rahmen %4,%5 %6x%7")
				                .arg(QLatin1String(what), objName(w)).arg(int(w->windowType()), 0, 16).arg(r.x()).arg(r.y()).arg(r.width()).arg(r.height()));
			}
			break;
		case QEvent::ApplicationStateChange:
			rjdiag::log(QStringLiteral("[qt] Anwendungszustand %1").arg(int(static_cast<QApplicationStateChangeEvent*>(e)->applicationState())));
			break;
		case QEvent::ApplicationActivate:
		case QEvent::ApplicationDeactivate:
			rjdiag::log(QStringLiteral("[qt] Anwendung %1").arg(e->type() == QEvent::ApplicationActivate ? QStringLiteral("aktiviert") : QStringLiteral("deaktiviert")));
			break;
		default:
			break;
		}
		return false;
	}
};

QString snapshot()
{
	QStringList s;
	s << QStringLiteral("== Umgebung");
	s << QStringLiteral("Programm: %1, Prozess %2").arg(QCoreApplication::applicationFilePath()).arg(QCoreApplication::applicationPid());
	s << QStringLiteral("Qt %1, Plattform %2, System %3 (Kernel %4, %5)")
	         .arg(QLatin1String(qVersion()), QGuiApplication::platformName(), QSysInfo::prettyProductName(), QSysInfo::kernelVersion(), QSysInfo::buildAbi());
	for (QScreen* sc : QGuiApplication::screens())
	{
		const QRect g = sc->geometry();
		s << QStringLiteral("Bildschirm %1: %2,%3 %4x%5, Faktor %6, %7 dpi%8")
		         .arg(sc->name()).arg(g.x()).arg(g.y()).arg(g.width()).arg(g.height()).arg(sc->devicePixelRatio()).arg(sc->logicalDotsPerInch())
		         .arg(sc == QGuiApplication::primaryScreen() ? QStringLiteral(" (Haupt)") : QString());
	}
#ifdef Q_OS_WIN
	DWORD session = 0;
	ProcessIdToSessionId(GetCurrentProcessId(), &session);
	s << QStringLiteral("Remotesitzung: %1, Sitzung %2, Konsolensitzung %3")
	         .arg(GetSystemMetrics(SM_REMOTESESSION) ? QStringLiteral("ja") : QStringLiteral("nein")).arg(session).arg(WTSGetActiveConsoleSessionId());
#endif

	s << QString() << QStringLiteral("== Qt-Zustand");
	s << QStringLiteral("Anwendungszustand %1").arg(int(QGuiApplication::applicationState()));
	s << QStringLiteral("aktives Fenster %1").arg(objName(QApplication::activeWindow()));
	s << QStringLiteral("aktives Popup %1").arg(objName(QApplication::activePopupWidget()));
	s << QStringLiteral("Fokus-Widget %1").arg(objName(QApplication::focusWidget()));
	s << QStringLiteral("Fokus-Fenster %1").arg(windowName(QGuiApplication::focusWindow()));
	s << QStringLiteral("Mausfang (Qt) %1, Tastaturfang (Qt) %2").arg(objName(QWidget::mouseGrabber()), objName(QWidget::keyboardGrabber()));
	s << QStringLiteral("Umschalttasten: Qt gemerkt %1, Qt abgefragt %2, Maustasten %3")
	         .arg(mods(QGuiApplication::keyboardModifiers()), mods(QGuiApplication::queryKeyboardModifiers())).arg(int(QGuiApplication::mouseButtons()));
	s << QStringLiteral("Sichtbare Qt-Fenster:");
	for (QWidget* w : QApplication::topLevelWidgets())
		if (w->isVisible())
		{
			const QRect r = w->frameGeometry();
			s << QStringLiteral("  %1 Typ 0x%2 Rahmen %3,%4 %5x%6%7").arg(objName(w)).arg(int(w->windowType()), 0, 16).arg(r.x()).arg(r.y()).arg(r.width()).arg(r.height())
			         .arg(w->isActiveWindow() ? QStringLiteral(" aktiv") : QString());
		}
	if (auto* combo = qobject_cast<QComboBox*>(QApplication::focusWidget()))
	{
		const QRect r(combo->mapToGlobal(QPoint(0, 0)), combo->size());
		s << QStringLiteral("Auswahlliste im Fokus: %1, %2 Einträge, Feld %3,%4 %5x%6, Liste sichtbar %7")
		         .arg(objName(combo)).arg(combo->count()).arg(r.x()).arg(r.y()).arg(r.width()).arg(r.height()).arg(combo->view()->isVisible() ? 1 : 0);
	}

#ifdef Q_OS_WIN
	s << QString() << QStringLiteral("== Windows-Zustand");
	s << QStringLiteral("Vordergrund %1").arg(hwndInfo(GetForegroundWindow()));
	s << QStringLiteral("aktiv (eigener Thread) %1").arg(hwndInfo(GetActiveWindow()));
	s << QStringLiteral("Fokus (eigener Thread) %1").arg(hwndInfo(GetFocus()));
	s << QStringLiteral("Mausfang (eigener Thread) %1").arg(hwndInfo(GetCapture()));
	s << keyStates();
	s << guiThreadInfo(0, QStringLiteral("Vordergrund-Thread"));
	s << guiThreadInfo(GetCurrentThreadId(), QStringLiteral("eigener Thread"));
	s << QStringLiteral("Eigene Fenster der obersten Ebene (mit fremden Kindfenstern):");
	EnumWindows(enumOwnTop, reinterpret_cast<LPARAM>(&s));
	s << QStringLiteral("Sichtbare Fenster aller Programme, oben zuerst (fremde ohne Titel):");
	ZCtx z{ &s, 0 };
	EnumWindows(enumZOrder, reinterpret_cast<LPARAM>(&z));
#endif
	return s.join(QLatin1Char('\n'));
}

void rotate()
{
	QDir d(diagDir());
	const QStringList files = d.entryList({ QStringLiteral("fall-*.txt") }, QDir::Files, QDir::Name);
	for (int i = 0; i + KeepIncidents < files.size(); ++i)
		d.remove(files.at(i));
}

} // namespace

namespace rjdiag
{
void log(const QString& line)
{
	const QString l = stamp() + QLatin1Char(' ') + line;
	QMutexLocker lock(&g_mutex);
	g_ring.push_back(l);
	while (g_ring.size() > RingMax)
		g_ring.pop_front();
	if (g_live)
	{
		g_live->write(l.toUtf8());
		g_live->write("\n");
		g_live->flush();
	}
}

QStringList recent(int n)
{
	QMutexLocker lock(&g_mutex);
	QStringList out;
	const int start = qMax(0, int(g_ring.size()) - n);
	for (int i = start; i < int(g_ring.size()); ++i)
		out << g_ring[size_t(i)];
	return out;
}

QString incident(const QString& reason)
{
	const QString snap = snapshot();
	const QString path = QDir(diagDir()).filePath(QStringLiteral("fall-%1.txt").arg(QDateTime::currentDateTime().toString(QStringLiteral("yyyyMMdd-HHmmss-zzz"))));
	log(QStringLiteral("[fall] %1 -> %2").arg(reason, path));
	QFile f(path);
	if (f.open(QIODevice::WriteOnly | QIODevice::Text))
	{
		QTextStream out(&f);
		out << "remotejoe Falldatei\n";
		out << "Zeit: " << QDateTime::currentDateTime().toString(Qt::ISODateWithMs) << "\n";
		out << "Grund: " << reason << "\n\n";
		out << snap << "\n\n== Verlauf (älteste zuerst)\n";
		QMutexLocker lock(&g_mutex);
		for (const QString& l : g_ring)
			out << l << "\n";
	}
	rotate();
	return path;
}

void init(QApplication& app)
{
	if (g_started)
		return;
	g_started = true;
	if (qEnvironmentVariableIsSet("RJ_DIAG"))
	{
		g_live = new QFile(QDir(diagDir()).filePath(QStringLiteral("laufend.log")));
		if (!g_live->open(QIODevice::Append | QIODevice::Text))
		{
			delete g_live;
			g_live = nullptr;
		}
	}
	log(QStringLiteral("[start] Aufzeichnung läuft, Qt %1, %2").arg(QLatin1String(qVersion()), QSysInfo::prettyProductName()));
	app.installEventFilter(new QtFilter(&app));
	QObject::connect(&app, &QApplication::focusChanged, &app, [](QWidget* from, QWidget* to) {
		log(QStringLiteral("[qt] Fokus %1 -> %2").arg(objName(from), objName(to)));
	});
	QObject::connect(&app, &QGuiApplication::focusWindowChanged, &app, [](QWindow* w) { log(QStringLiteral("[qt] Fokusfenster %1").arg(windowName(w))); });
#ifdef Q_OS_WIN
	app.installNativeEventFilter(new NativeFilter);
	SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_DESKTOPSWITCH, nullptr, winEventHook, 0, 0, WINEVENT_OUTOFCONTEXT);
	SetWinEventHook(EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE, nullptr, winEventHook, 0, 0, WINEVENT_OUTOFCONTEXT);
	SetWinEventHook(EVENT_OBJECT_FOCUS, EVENT_OBJECT_FOCUS, nullptr, winEventHook, 0, 0, WINEVENT_OUTOFCONTEXT);
#endif
}

void attachShortcut(QWidget* mainWindow)
{
	auto* sc = new QShortcut(QKeySequence(QStringLiteral("Ctrl+Shift+F12")), mainWindow);
	sc->setContext(Qt::ApplicationShortcut);
	QObject::connect(sc, &QShortcut::activated, mainWindow, [mainWindow] {
		const QString p = incident(QStringLiteral("von Hand (Strg+Umschalt+F12)"));
		QMessageBox::information(mainWindow, QStringLiteral("remotejoe"), QStringLiteral("Diagnose gesichert:\n%1").arg(QDir::toNativeSeparators(p)));
	});
}
} // namespace rjdiag
