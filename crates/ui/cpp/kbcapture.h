#pragma once

class QWidget;

/// Tastenkürzel des Desktops (Windows-Taste, Alt+Tab …) an eine Sitzung geben, solange sie den
/// Tastaturfokus hat. Wayland: Protokoll „keyboard-shortcuts-inhibit“ (der Desktop fragt beim
/// ersten Mal selbst nach); X11: Tastatur-Grab des Fensters. Windows: nichts nötig.
namespace KeyCapture {

/// Für das Fenster von `w` einschalten (ersetzt eine frühere Freigabe).
void capture(QWidget* w);
/// Wieder an den Desktop zurückgeben.
void release();
/// Ist gerade eine Freigabe aktiv?
bool active();
/// Antworten des Desktops abholen (Diagnose).
void pump();
/// Diagnoseprotokoll eingeschaltet (RJ_KEYLOG)?
bool logging();

} // namespace KeyCapture
