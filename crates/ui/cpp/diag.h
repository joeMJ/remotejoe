#pragma once

// Fehlerdiagnose: zeichnet laufend im Speicher auf, was um Fenster, Fokus, Maus und Auswahllisten geschieht
// (Qt-Ereignisse, unter Windows zusätzlich Fensternachrichten und systemweite Vordergrundwechsel), und schreibt
// im Fehlerfall eine Falldatei mit diesem Verlauf und einer Momentaufnahme des Fenster- und Eingabezustands.
// Fremde Fenstertitel und gedrückte Zeichentasten werden nicht aufgezeichnet (nur Programm, Klasse und
// Umschalt-/Strg-/Alt-/Windows-Tasten).
//
// Falldateien: %TEMP%/remotejoe-diag/fall-*.txt (die neuesten 20 bleiben). Mit RJ_DIAG=1 zusätzlich laufend
// nach %TEMP%/remotejoe-diag/laufend.log. Strg+Umschalt+F12 schreibt jederzeit eine Falldatei von Hand.

#include <QtCore/QString>

class QApplication;
class QWidget;

namespace rjdiag
{
/// Aufzeichnung starten (einmal, direkt nach dem Anlegen der QApplication).
void init(QApplication& app);
/// Tastenkürzel für eine Falldatei von Hand am Hauptfenster anbringen.
void attachShortcut(QWidget* mainWindow);
/// Eine Zeile in den Verlauf.
void log(const QString& line);
/// Falldatei schreiben: Grund, Momentaufnahme, Verlauf. Gibt den Pfad zurück.
QString incident(const QString& reason);
/// Die letzten `n` Zeilen des Verlaufs (für „Fehler melden“).
QStringList recent(int n);
} // namespace rjdiag
