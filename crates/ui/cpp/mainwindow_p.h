// Interne Hilfen des Hauptfensters für die Moduldateien (cpp/modules/…): Rollen der Baumeinträge, Texte, JSON,
// Symbole. Nicht für andere Teile des Programms.
#pragma once

#include <QtCore/QJsonObject>
#include <QtCore/QString>
#include <QtGui/QIcon>

class RjBackend;

namespace rjmw {

inline constexpr int RoleId = Qt::UserRole + 1;
inline constexpr int RoleFolder = Qt::UserRole + 2;
/// Gast eines Hypervisors (Kennung), Ziel des Hypervisors, Konsolenadresse.
inline constexpr int RoleGuest = Qt::UserRole + 3;
inline constexpr int RoleGuestTarget = Qt::UserRole + 4;
inline constexpr int RoleConsoleHost = Qt::UserRole + 5;
/// Platzhalter unter einem Hypervisor, bis die Gäste geladen sind.
inline constexpr int RolePlaceholder = Qt::UserRole + 6;
inline constexpr int RoleProtocol = Qt::UserRole + 7;
/// Ampelfarbe aus dem Baum (für das Neuzeichnen mit Sitzungspunkt).
inline constexpr int RoleLight = Qt::UserRole + 8;
/// Zuletzt gezeichneter Sitzungspunkt (0, 1, 2).
inline constexpr int RoleSessionMark = Qt::UserRole + 9;
/// Anmeldeobjekt (keine Verbindung; öffnet die Bearbeitung).
inline constexpr int RoleLogin = Qt::UserRole + 10;
/// Zustand eines Gasts (z. B. `running`), für die Übersicht.
inline constexpr int RoleGuestState = Qt::UserRole + 11;
/// Doppelklick öffnet einen anderen Zugangsweg als den Hauptweg.
inline constexpr int RoleOtherWay = Qt::UserRole + 40;
// Ordner eines Moduls (eigenes Symbol, Einfachklick öffnet die Karte des Moduls).
inline constexpr int RoleSite = Qt::UserRole + 41;
/// Virtueller Switch (Name; kein Ziel, öffnet den Tab „Virtueller Switch“).
inline constexpr int RoleVSwitch = Qt::UserRole + 42;
// Symbolart: Protokoll oder erkannte Geräteart (aus dem Inventar).
inline constexpr int RoleIconKind = Qt::UserRole + 12;
/// Fach „Makros“ im Baum: 1 = das Fach selbst, 2 = ein Makro (RoleId = Makro-ID).
inline constexpr int RoleMacro = Qt::UserRole + 20;

QString text(RjBackend* b, const char* key);
QJsonObject parseJson(const QString& s);
QIcon icon(const QString& kind, const QString& light, bool folder, int session = 0);

} // namespace rjmw

QIcon rjVirtualSwitchIcon();
