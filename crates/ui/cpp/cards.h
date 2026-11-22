#pragma once

// Bausteine im Kartenstil (Übersicht und Einstellungen): Karte, Abschnittsüberschrift, Kachel,
// Plakette, klickbare Zeile und ein umbrechendes Layout. Das Aussehen kommt aus dem Stylesheet
// des Hauptfensters (Objektnamen rjCard, rjSection, rjTile, rjRow; Eigenschaft rjPill).

#include <QtWidgets/QFrame>
#include <QtCore/QHash>
#include <QtWidgets/QTreeWidget>
#include <QtWidgets/QLayout>
#include <QtCore/QList>
#include <QtGui/QIcon>
#include <QtCore/QJsonObject>

#include <functional>

class QLabel;
class QPushButton;
class QMenu;
class QTableWidget;
class QVBoxLayout;

namespace rjui
{
/// Karte mit abgerundetem Rahmen; `inner` erhält das innere Layout.
QFrame* card(QWidget* parent, QVBoxLayout** inner);
/// Abschnittsüberschrift in Großbuchstaben, gedämpft.
QLabel* section(const QString& text, QWidget* parent);
/// Kennzahl-Kachel: kleine Beschriftung, darunter der Wert (`value`).
QFrame* tile(const QString& label, QLabel** value, QWidget* parent);
/// Plakette; `kind`: ok, warn, err, off, accent.
QLabel* pill(const QString& text, const QString& kind, QWidget* parent);
void setPill(QLabel* pill, const QString& text, const QString& kind);
/// Gezeichnete Symbole für Knöpfe, in der Textfarbe der aktuellen Palette (ausgegraut bei gesperrten Knöpfen):
/// Zettel (Protokolle), Funkwellen (abfragen), Lupe (Analyse), Bücher im Regal (Inventar), Pfeilkreis (Neustart),
/// Zahnrad (Aktualisieren), Person (Nutzer anzeigen).
enum class Glyph
{
	Note,
	Signal,
	Search,
	Library,
	Refresh,
	Gear,
	Person,
	/// Leiste: Netzplan, Stecker, Ebenen, Weg, Geräte, Zeilen, Etikett, Einpassen, Originalgröße.
	Topology,
	Cable,
	Layers,
	Route,
	Devices,
	Rows,
	Tag,
	Fit,
	ActualSize,
	/// Maximieren (Rahmen mit Pfeil nach außen), alle aufklappen (Plus in Kästchen), alle einklappen (Minus in
	/// Kästchen), Drucken (Drucker mit Blatt).
	Maximize,
	ExpandAll,
	CollapseAll,
	Print,
	/// Sichern (Datenträger mit Pfeil nach unten).
	Backup,
	/// Kurzbeschriftung mit zwei Oktetten („.2.1“).
	Octets,
	/// Bericht (Blatt mit Zeilen und Haken).
	Report,
};
QIcon glyphIcon(Glyph g);
/// PoE-Symbol: Stecker (zwei Kontakte, Gehäuse, Kabel) in Farbe `c`.
QIcon plugIcon(const QColor& c);
/// PoE-Stufe eines Ports der Switch-Seite: „++“ PoE++ (Klasse 5 und höher bzw. über 30 W), „+“ PoE+ (Klasse 4 bzw.
/// über 15,4 W), sonst leer.
QString poeStage(const QJsonObject& port);
/// Knopf „Protokolle“ mit Zettel-Symbol und Menü: Systemlog und Sicherheitslog, ganz oder gefiltert.
/// `text` liefert die Beschriftungen, `pick` erhält „system“ bzw. „security“ und ob gefiltert.
/// Tabelle zum Kopieren einrichten: Zeilen markierbar (auch mehrere), Strg+C und Kontextmenü „Kopieren“ bzw.
/// „Alles kopieren“ geben tabulatorgetrennten Text mit Kopfzeile in die Zwischenablage. Ein Rechtsklick wählt die
/// Zeile darunter; `extra` hängt weitere Einträge für diese Zeile (-1: keine) in dasselbe Menü.
void makeCopyable(QTableWidget* t, const std::function<QString(const char*)>& text, const std::function<void(QMenu*, int row)>& extra = {});

/// Zeile, die auf Klick reagiert (Bereichsliste); `selected` hebt sie hervor.
class ClickRow : public QFrame
{
	Q_OBJECT
public:
	explicit ClickRow(QWidget* parent = nullptr);
	void setSelected(bool on);

signals:
	void clicked();

protected:
	void mouseReleaseEvent(QMouseEvent* e) override;
	void keyPressEvent(QKeyEvent* e) override;
};

/// Schlanker Bereichsbaum (Einstellungen, Eigenschaften): Gruppen halbfett, Einträge eingerückt in normaler Schrift,
/// je Zeile ein Symbol (Font Awesome); rechts optional eine kleine Markierung („geändert“, „geerbt“).
class AreaTree : public QTreeWidget
{
public:
	explicit AreaTree(QWidget* parent = nullptr);
	/// Gruppe (oben oder unter `parent`); ein Klick wählt ihren ersten Eintrag bzw. `key`, wenn gesetzt.
	QTreeWidgetItem* addGroup(const QString& glyph, const QString& text, QTreeWidgetItem* parent = nullptr, const QString& key = QString());
	/// Eintrag mit Schlüssel `key` unter `parent` (nullptr = oberste Ebene).
	QTreeWidgetItem* addEntry(QTreeWidgetItem* parent, const QString& key, const QString& glyph, const QString& text);
	/// Markierung rechts; `warn` färbt sie in der Warnfarbe.
	void setMark(const QString& key, const QString& text, bool warn = false);
	/// Eintrag auswählen, ohne `onSelect` auszulösen.
	void selectKey(const QString& key);
	QTreeWidgetItem* itemFor(const QString& key) const { return m_items.value(key); }
	void clearAll();
	/// Gewählter Eintrag (Schlüssel, Zeile).
	std::function<void(const QString&, QTreeWidgetItem*)> onSelect;

private:
	QHash<QString, QTreeWidgetItem*> m_items;
	bool m_quiet = false;
};

/// Fläche mit umbrechendem Layout; passt ihre Höhe bei jeder Breitenänderung an die Zahl der Zeilen an
/// (in einer Bildlauffläche reicht Qt die breitenabhängige Höhe sonst nicht weiter).
class FlowBox : public QWidget
{
public:
	explicit FlowBox(QWidget* parent, int spacing = 6);
	class FlowLayout* flow() const { return m_flow; }
	/// Nach dem Hinzufügen oder Entfernen von Elementen aufrufen.
	void relayout();

protected:
	void resizeEvent(QResizeEvent* e) override;

private:
	class FlowLayout* m_flow;
};

/// Layout, das seine Elemente zeilenweise umbricht (Plaketten der Dienste).
class FlowLayout : public QLayout
{
public:
	explicit FlowLayout(QWidget* parent, int spacing = 6);
	~FlowLayout() override;
	void addItem(QLayoutItem* item) override;
	int count() const override;
	QLayoutItem* itemAt(int index) const override;
	QLayoutItem* takeAt(int index) override;
	Qt::Orientations expandingDirections() const override;
	bool hasHeightForWidth() const override;
	int heightForWidth(int width) const override;
	QSize minimumSize() const override;
	QSize sizeHint() const override;
	void setGeometry(const QRect& rect) override;

private:
	int doLayout(const QRect& rect, bool testOnly) const;
	QList<QLayoutItem*> m_items;
	int m_spacing;
};
} // namespace rjui
