#pragma once

// Bausteine im Kartenstil (Übersicht und Einstellungen): Karte, Abschnittsüberschrift, Kachel,
// Plakette, klickbare Zeile und ein umbrechendes Layout. Das Aussehen kommt aus dem Stylesheet
// des Hauptfensters (Objektnamen rjCard, rjSection, rjTile, rjRow; Eigenschaft rjPill).

#include <QtWidgets/QFrame>
#include <QtWidgets/QLayout>
#include <QtCore/QList>

class QLabel;
class QVBoxLayout;

namespace rjui
{
/// Karte mit abgerundetem Rahmen; `inner` erhält das innere Layout.
QFrame* card(QWidget* parent, QVBoxLayout** inner);
/// Abschnittsüberschrift in Großbuchstaben, gedämpft.
QLabel* section(const QString& text, QWidget* parent);
/// Kennzahl-Kachel: kleine Beschriftung, darunter der Wert (`value`).
QFrame* tile(const QString& label, QLabel** value, QWidget* parent);
/// Plakette; `kind`: ok, warn, off, accent.
QLabel* pill(const QString& text, const QString& kind, QWidget* parent);
void setPill(QLabel* pill, const QString& text, const QString& kind);

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
