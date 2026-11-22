#pragma once
// Symbole aus Font Awesome Free 6.7.2 (CC BY 4.0, Fonticons, Inc.), als SVG unter :/fa/ eingebettet.
// Namen wie in Font Awesome; die Umrissform („regular“) trägt das Präfix „r-“, z. B. „r-star“.

#include <QColor>
#include <QIcon>
#include <QRectF>
#include <QString>

class QPainter;

/// Symbol, das bei jedem Zeichnen die aktuelle Palette nimmt: gedämpfte Textfarbe, in ausgewählten Zeilen die
/// Textfarbe der Auswahl, gesperrt die gesperrte Textfarbe. Folgt so jedem Wechsel des Farbschemas.
QIcon faIcon(const QString& name);
/// Symbol in fester Farbe `color`; `selected` (falls gültig) für ausgewählte Zeilen.
QIcon faIcon(const QString& name, const QColor& color, const QColor& selected = QColor());
/// Gedämpfte Symbolfarbe der aktuellen Palette (Text, leicht zum Hintergrund gemischt).
QColor faMutedColor();
/// Malt das Symbol `name` in `color` eingepasst und mittig in `rect`.
void faPaint(QPainter* p, const QRectF& rect, const QString& name, const QColor& color);
/// Pfad einer SVG-Datei des Symbols in `color` (für Stylesheets, z. B. `image: url(...)`); einmal je Farbe im
/// Zwischenspeicher-Ordner angelegt. Leer, wenn nichts geschrieben werden konnte.
QString faFile(const QString& name, const QColor& color);
/// Gibt es das Symbol?
bool faHas(const QString& name);
