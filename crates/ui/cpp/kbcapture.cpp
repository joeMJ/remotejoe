#include "kbcapture.h"

#include <QtCore/QPointer>
#include <QtGui/QGuiApplication>
#include <QtGui/QWindow>
#include <QtWidgets/QWidget>

#if defined(Q_OS_LINUX) && QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
#include <QtGui/qguiapplication_platform.h>
#include <QtCore/QByteArray>
#include <cstdint>
#include <cstring>
#include <dlfcn.h>
#define RJ_WAYLAND_INHIBIT 1
// Undurchsichtige Typen von libwayland (global, wie in den Qt-Kopfdateien).
struct wl_display;
struct wl_proxy;
struct wl_event_queue;
#endif

namespace {

QPointer<QWindow> s_grabbed;

/// Diagnose (Start mit RJ_KEYLOG=1): Freigabe und Tastendrücke auf stderr, ohne Textinhalte.
bool keylog()
{
	static const bool on = qEnvironmentVariableIsSet("RJ_KEYLOG");
	return on;
}
bool s_active = false;

#ifdef RJ_WAYLAND_INHIBIT

// Minimale Anbindung an libwayland-client, zur Laufzeit geladen: Der Bau braucht keine
// Wayland-Pakete, und ohne Wayland bleibt alles beim Alten. Die Strukturen entsprechen
// wayland-util.h (stabile Schnittstelle der Bibliothek).
struct RjWlMessage
{
	const char* name;
	const char* signature;
	const struct RjWlInterface** types;
};
struct RjWlInterface
{
	const char* name;
	int version;
	int method_count;
	const RjWlMessage* methods;
	int event_count;
	const RjWlMessage* events;
};

using MarshalFlags = wl_proxy* (*)(wl_proxy*, uint32_t, const RjWlInterface*, uint32_t, uint32_t, ...);
using AddListener = int (*)(wl_proxy*, void (**)(void), void*);
using CreateQueue = wl_event_queue* (*)(wl_display*);
using SetQueue = void (*)(wl_proxy*, wl_event_queue*);
using RoundtripQueue = int (*)(wl_display*, wl_event_queue*);
using Flush = int (*)(wl_display*);
using GetVersion = uint32_t (*)(wl_proxy*);
using CreateWrapper = void* (*)(void*);
using WrapperDestroy = void (*)(void*);
using GetClass = const char* (*)(wl_proxy*);
using DispatchPending = int (*)(wl_display*, wl_event_queue*);

struct Lib
{
	bool ok = false;
	MarshalFlags marshal = nullptr;
	AddListener addListener = nullptr;
	CreateQueue createQueue = nullptr;
	SetQueue setQueue = nullptr;
	RoundtripQueue roundtrip = nullptr;
	Flush flush = nullptr;
	GetVersion version = nullptr;
	CreateWrapper wrapper = nullptr;
	WrapperDestroy wrapperDestroy = nullptr;
	GetClass className = nullptr;
	DispatchPending dispatchPending = nullptr;
	const RjWlInterface* registryIf = nullptr;
	const RjWlInterface* surfaceIf = nullptr;
	const RjWlInterface* seatIf = nullptr;
};

Lib& lib()
{
	static Lib l = [] {
		Lib x;
		void* h = dlopen("libwayland-client.so.0", RTLD_NOW | RTLD_NOLOAD);
		if (!h)
			h = dlopen("libwayland-client.so.0", RTLD_NOW);
		if (!h)
			return x;
		x.marshal = reinterpret_cast<MarshalFlags>(dlsym(h, "wl_proxy_marshal_flags"));
		x.addListener = reinterpret_cast<AddListener>(dlsym(h, "wl_proxy_add_listener"));
		x.createQueue = reinterpret_cast<CreateQueue>(dlsym(h, "wl_display_create_queue"));
		x.setQueue = reinterpret_cast<SetQueue>(dlsym(h, "wl_proxy_set_queue"));
		x.roundtrip = reinterpret_cast<RoundtripQueue>(dlsym(h, "wl_display_roundtrip_queue"));
		x.flush = reinterpret_cast<Flush>(dlsym(h, "wl_display_flush"));
		x.version = reinterpret_cast<GetVersion>(dlsym(h, "wl_proxy_get_version"));
		x.wrapper = reinterpret_cast<CreateWrapper>(dlsym(h, "wl_proxy_create_wrapper"));
		x.wrapperDestroy = reinterpret_cast<WrapperDestroy>(dlsym(h, "wl_proxy_wrapper_destroy"));
		x.className = reinterpret_cast<GetClass>(dlsym(h, "wl_proxy_get_class"));
		x.dispatchPending = reinterpret_cast<DispatchPending>(dlsym(h, "wl_display_dispatch_queue_pending"));
		x.registryIf = static_cast<const RjWlInterface*>(dlsym(h, "wl_registry_interface"));
		x.surfaceIf = static_cast<const RjWlInterface*>(dlsym(h, "wl_surface_interface"));
		x.seatIf = static_cast<const RjWlInterface*>(dlsym(h, "wl_seat_interface"));
		x.ok = x.marshal && x.addListener && x.createQueue && x.setQueue && x.roundtrip && x.flush && x.version && x.wrapper && x.wrapperDestroy && x.className && x.dispatchPending && x.registryIf && x.surfaceIf && x.seatIf;
		return x;
	}();
	return l;
}

// Protokoll keyboard-shortcuts-inhibit-unstable-v1 (wayland-protocols), von Hand beschrieben.
extern const RjWlInterface inhibitorIf;
const RjWlMessage inhibitorRequests[] = { { "destroy", "", nullptr } };
const RjWlMessage inhibitorEvents[] = { { "active", "", nullptr }, { "inactive", "", nullptr } };
const RjWlInterface inhibitorIf = { "zwp_keyboard_shortcuts_inhibitor_v1", 1, 1, inhibitorRequests, 2, inhibitorEvents };
const RjWlInterface* inhibitTypes[3] = { &inhibitorIf, nullptr, nullptr };
const RjWlMessage managerRequests[] = { { "destroy", "", nullptr }, { "inhibit_shortcuts", "noo", inhibitTypes } };
const RjWlInterface managerIf = { "zwp_keyboard_shortcuts_inhibit_manager_v1", 1, 2, managerRequests, 0, nullptr };

constexpr uint32_t WL_MARSHAL_FLAG_DESTROY = 1;

wl_proxy* s_manager = nullptr;
wl_proxy* s_inhibitor = nullptr;
bool s_managerLooked = false;
wl_event_queue* s_queue = nullptr;
/// Hat der Desktop die Freigabe als aktiv gemeldet?
bool s_desktopActive = false;

void onActive(void*, wl_proxy*)
{
	s_desktopActive = true;
	if (keylog())
		qInfo("[remotejoe-tasten] Desktop meldet: Tastenkürzel gehen an remotejoe (aktiv)");
}

void onInactive(void*, wl_proxy*)
{
	s_desktopActive = false;
	if (keylog())
		qInfo("[remotejoe-tasten] Desktop meldet: Tastenkürzel wieder beim Desktop (inaktiv)");
}

void onGlobal(void*, wl_proxy* registry, uint32_t name, const char* interface, uint32_t version)
{
	Lib& l = lib();
	if (s_manager || std::strcmp(interface, managerIf.name) != 0)
		return;
	// wl_registry.bind (Opcode 0, Signatur „usun“).
	s_manager = l.marshal(registry, 0, &managerIf, version < 1 ? version : 1, 0, name, managerIf.name, 1u, nullptr);
}

void onGlobalRemove(void*, wl_proxy*, uint32_t) {}

/// Holt den Manager einmalig über eine eigene Ereigniswarteschlange (stört Qt nicht).
wl_proxy* manager(wl_display* display)
{
	if (s_managerLooked)
		return s_manager;
	s_managerLooked = true;
	Lib& l = lib();
	if (!l.ok)
		return nullptr;
	inhibitTypes[1] = l.surfaceIf;
	inhibitTypes[2] = l.seatIf;
	wl_event_queue* queue = l.createQueue(display);
	if (!queue)
		return nullptr;
	s_queue = queue;
	// wl_display.get_registry (Opcode 1) über eine Hülle mit eigener Warteschlange, damit der
	// Ereignisfaden von Qt die Antworten nicht abholt.
	auto* wrapped = static_cast<wl_proxy*>(l.wrapper(display));
	if (!wrapped)
		return nullptr;
	l.setQueue(wrapped, queue);
	wl_proxy* registry = l.marshal(wrapped, 1, l.registryIf, l.version(wrapped), 0, nullptr);
	l.wrapperDestroy(wrapped);
	if (!registry)
		return nullptr;
	static void (*listener[2])(void) = { reinterpret_cast<void (*)(void)>(&onGlobal), reinterpret_cast<void (*)(void)>(&onGlobalRemove) };
	l.addListener(registry, listener, nullptr);
	l.roundtrip(display, queue);
	if (s_manager)
		l.setQueue(s_manager, queue);
	if (keylog())
		qInfo("[remotejoe-tasten] Wayland: Freigabe-Schnittstelle des Desktops %s", s_manager ? "gefunden" : "NICHT vorhanden");
	return s_manager;
}

void dropInhibitor()
{
	if (!s_inhibitor)
		return;
	Lib& l = lib();
	// zwp_keyboard_shortcuts_inhibitor_v1.destroy (Opcode 0).
	l.marshal(s_inhibitor, 0, nullptr, l.version(s_inhibitor), WL_MARSHAL_FLAG_DESTROY);
	s_inhibitor = nullptr;
	s_desktopActive = false;
	if (auto* app = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>())
		l.flush(app->display());
}

bool waylandInhibit(QWindow* window)
{
	auto* app = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>();
	if (!app || !window)
		return false;
	wl_display* display = reinterpret_cast<wl_display*>(app->display());
	wl_proxy* m = manager(display);
	auto* surface = reinterpret_cast<wl_proxy*>(window->winId());
	auto* seat = reinterpret_cast<wl_proxy*>(app->seat());
	if (!m || !surface || !seat)
		return false;
	Lib& lc = lib();
	// Unter Qt Wayland ist die Fensterkennung die wl_surface; zur Sicherheit prüfen.
	if (std::strcmp(lc.className(surface), "wl_surface") != 0 || std::strcmp(lc.className(seat), "wl_seat") != 0)
	{
		if (keylog())
			qInfo("[remotejoe-tasten] Wayland: Fenster- oder Sitzkennung unerwartet (%s, %s)", lc.className(surface), lc.className(seat));
		return false;
	}
	Lib& l = lib();
	// Gleiches Fenster: Die vorhandene Freigabe gilt weiter (der Desktop schaltet sie mit dem Fokus selbst).
	static wl_proxy* s_surface = nullptr;
	if (s_inhibitor && s_surface == surface)
	{
		l.dispatchPending(display, s_queue);
		// Schon aktiv: weiter so. Noch nicht aktiv (z. B. lief die erste Anforderung, während der
		// Desktop um Erlaubnis gefragt hat): neu anfordern, damit sie jetzt greift.
		if (s_desktopActive)
			return true;
		if (keylog())
			qInfo("[remotejoe-tasten] Wayland: Freigabe noch nicht aktiv, neu anfordern");
	}
	dropInhibitor();
	// zwp_keyboard_shortcuts_inhibit_manager_v1.inhibit_shortcuts (Opcode 1): neue Kennung, Oberfläche, Sitz.
	s_inhibitor = l.marshal(m, 1, &inhibitorIf, l.version(m), 0, nullptr, surface, seat);
	s_surface = surface;
	if (s_inhibitor)
	{
		static void (*listener[2])(void) = { reinterpret_cast<void (*)(void)>(&onActive), reinterpret_cast<void (*)(void)>(&onInactive) };
		l.addListener(s_inhibitor, listener, nullptr);
	}
	l.flush(display);
	if (keylog())
		qInfo("[remotejoe-tasten] Wayland: Freigabe angefordert (%s)", s_inhibitor ? "ok" : "fehlgeschlagen");
	return s_inhibitor != nullptr;
}

#endif

} // namespace

namespace KeyCapture {

void capture(QWidget* w)
{
	if (!w)
		return;
	QWindow* window = w->window()->windowHandle();
	if (!window)
		return;
	const QString platform = QGuiApplication::platformName();
#ifdef RJ_WAYLAND_INHIBIT
	if (platform.startsWith(QLatin1String("wayland")))
	{
		s_active = waylandInhibit(window);
		return;
	}
#endif
	if (platform == QLatin1String("xcb"))
	{
		if (s_grabbed && s_grabbed != window)
			s_grabbed->setKeyboardGrabEnabled(false);
		s_active = window->setKeyboardGrabEnabled(true);
		s_grabbed = s_active ? window : nullptr;
		if (keylog())
			qInfo("[remotejoe-tasten] X11: Tastatur-Grab %s", s_active ? "ok" : "fehlgeschlagen");
	}
}

void pump()
{
#ifdef RJ_WAYLAND_INHIBIT
	// Antworten des Desktops (aktiv/inaktiv) abholen.
	if (s_inhibitor && s_queue)
		if (auto* app = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>())
			lib().dispatchPending(reinterpret_cast<wl_display*>(app->display()), s_queue);
#endif
}

bool logging()
{
	return keylog();
}

void release()
{
	if (keylog() && s_active)
		qInfo("[remotejoe-tasten] Freigabe zurückgegeben");
#ifdef RJ_WAYLAND_INHIBIT
	dropInhibitor();
#endif
	if (s_grabbed)
		s_grabbed->setKeyboardGrabEnabled(false);
	s_grabbed = nullptr;
	s_active = false;
}

bool active()
{
	return s_active;
}

} // namespace KeyCapture
