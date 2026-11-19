#include <array>
#include "cards.h"

#include <QtGui/QPixmap>

#include <cmath>

#include <QtCore/QtMath>
#include <QtGui/QIconEngine>
#include <QtGui/QKeyEvent>
#include <QtGui/QMouseEvent>
#include <QtGui/QPainter>
#include <QtGui/QPainterPath>
#include <QtGui/QResizeEvent>
#include <QtGui/QClipboard>
#include <QtGui/QShortcut>
#include <QtWidgets/QApplication>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QLabel>
#include <QtWidgets/QMenu>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QStyle>
#include <QtWidgets/QVBoxLayout>
#include <QtWidgets/QWidget>

namespace rjui
{
QFrame* card(QWidget* parent, QVBoxLayout** inner)
{
	auto* f = new QFrame(parent);
	f->setObjectName(QStringLiteral("rjCard"));
	auto* l = new QVBoxLayout(f);
	l->setContentsMargins(14, 10, 14, 12);
	l->setSpacing(6);
	if (inner)
		*inner = l;
	return f;
}

QLabel* section(const QString& text, QWidget* parent)
{
	auto* l = new QLabel(text.toUpper(), parent);
	l->setObjectName(QStringLiteral("rjSection"));
	QFont f = l->font();
	f.setPointSizeF(f.pointSizeF() * 0.85);
	f.setBold(true);
	f.setLetterSpacing(QFont::AbsoluteSpacing, 0.6);
	l->setFont(f);
	return l;
}

QFrame* tile(const QString& label, QLabel** value, QWidget* parent)
{
	auto* f = new QFrame(parent);
	f->setObjectName(QStringLiteral("rjTile"));
	auto* l = new QVBoxLayout(f);
	l->setContentsMargins(10, 6, 10, 8);
	l->setSpacing(2);
	auto* cap = new QLabel(label, f);
	cap->setObjectName(QStringLiteral("rjTileLabel"));
	auto* val = new QLabel(f);
	QFont vf = val->font();
	vf.setPointSizeF(vf.pointSizeF() * 1.25);
	vf.setBold(true);
	val->setFont(vf);
	val->setTextInteractionFlags(Qt::TextSelectableByMouse);
	l->addWidget(cap);
	l->addWidget(val);
	if (value)
		*value = val;
	return f;
}

QLabel* pill(const QString& text, const QString& kind, QWidget* parent)
{
	auto* l = new QLabel(parent);
	setPill(l, text, kind);
	return l;
}

void setPill(QLabel* l, const QString& text, const QString& kind)
{
	l->setText(text);
	l->setVisible(!text.isEmpty());
	if (l->property("rjPill").toString() != kind)
	{
		l->setProperty("rjPill", kind);
		l->style()->unpolish(l);
		l->style()->polish(l);
	}
}

namespace
{
// Zeichnet bei jedem Aufruf neu, damit die Symbole einem Wechsel zwischen Hell und Dunkel folgen.
// Die Formen sind in Anteilen eines Quadrats der Kantenlänge s beschrieben.
class GlyphEngine : public QIconEngine
{
public:
	explicit GlyphEngine(Glyph g)
	  : m_glyph(g)
	{
	}
	void paint(QPainter* p, const QRect& rect, QIcon::Mode mode, QIcon::State) override
	{
		const QColor c = QApplication::palette().color(mode == QIcon::Disabled ? QPalette::Disabled : QPalette::Active, QPalette::ButtonText);
		const qreal s = qMin(rect.width(), rect.height());
		const QPointF o(rect.x() + (rect.width() - s) / 2.0, rect.y() + (rect.height() - s) / 2.0);
		auto P = [&](qreal x, qreal y) { return o + QPointF(x * s, y * s); };
		auto onCircle = [&](QPointF m, qreal r, qreal deg) {
			const qreal a = qDegreesToRadians(deg);
			return QPointF(m.x() + r * s * std::cos(a), m.y() - r * s * std::sin(a));
		};
		p->save();
		p->setRenderHint(QPainter::Antialiasing);
		QPen pen(c, qMax<qreal>(1.0, s / 14.0));
		pen.setJoinStyle(Qt::RoundJoin);
		pen.setCapStyle(Qt::RoundCap);
		p->setPen(pen);
		switch (m_glyph)
		{
		case Glyph::Note:
		{
			const qreal x0 = 0.14, y0 = 0.05, x1 = 0.86, y1 = 0.95, fold = 0.26;
			QPainterPath page;
			page.moveTo(P(x0, y0));
			page.lineTo(P(x1 - fold, y0));
			page.lineTo(P(x1, y0 + fold));
			page.lineTo(P(x1, y1));
			page.lineTo(P(x0, y1));
			page.closeSubpath();
			p->drawPath(page);
			QPainterPath ear;
			ear.moveTo(P(x1 - fold, y0));
			ear.lineTo(P(x1 - fold, y0 + fold));
			ear.lineTo(P(x1, y0 + fold));
			p->drawPath(ear);
			for (int i = 0; i < 3; ++i)
			{
				const qreal y = y0 + 0.9 * (0.45 + 0.17 * i);
				p->drawLine(P(x0 + 0.16, y), P(i == 2 ? 0.54 : 0.70, y));
			}
			break;
		}
		case Glyph::Signal:
		{
			// Mast mit Funkwellen nach links und rechts.
			const QPointF m = P(0.5, 0.38);
			p->drawLine(P(0.36, 0.94), m);
			p->drawLine(P(0.64, 0.94), m);
			for (qreal r : { 0.17, 0.33 })
			{
				const QRectF box(m.x() - r * s, m.y() - r * s, 2 * r * s, 2 * r * s);
				p->drawArc(box, -40 * 16, 80 * 16);
				p->drawArc(box, 140 * 16, 80 * 16);
			}
			p->setBrush(c);
			p->drawEllipse(m, s * 0.05, s * 0.05);
			break;
		}
		case Glyph::Search:
		{
			const QPointF m = P(0.42, 0.42);
			p->drawEllipse(m, s * 0.27, s * 0.27);
			QPen thick = pen;
			thick.setWidthF(pen.widthF() * 1.8);
			p->setPen(thick);
			p->drawLine(onCircle(m, 0.29, -45), P(0.88, 0.88));
			break;
		}
		case Glyph::Library:
		{
			// Zwei stehende Bücher, ein angelehntes, darunter das Regalbrett.
			p->drawRect(QRectF(P(0.10, 0.22), P(0.30, 0.86)));
			p->drawRect(QRectF(P(0.34, 0.12), P(0.54, 0.86)));
			p->drawLine(P(0.10, 0.34), P(0.30, 0.34));
			p->drawLine(P(0.34, 0.26), P(0.54, 0.26));
			QPolygonF lean;
			lean << P(0.60, 0.86) << P(0.76, 0.86) << P(0.92, 0.28) << P(0.76, 0.24);
			p->drawPolygon(lean);
			p->drawLine(P(0.04, 0.94), P(0.96, 0.94));
			break;
		}
		case Glyph::Refresh:
		{
			// Kreisbogen gegen den Uhrzeigersinn mit Pfeilspitze am Ende.
			const QPointF m = P(0.5, 0.52);
			const qreal r = 0.32, start = 40, span = 290;
			const QRectF box(m.x() - r * s, m.y() - r * s, 2 * r * s, 2 * r * s);
			p->drawArc(box, int(start * 16), int(span * 16));
			const qreal a = qDegreesToRadians(start + span);
			const QPointF end = onCircle(m, r, start + span);
			const QPointF tangent(-std::sin(a), -std::cos(a));
			const QPointF radial(std::cos(a), -std::sin(a));
			QPolygonF head;
			head << end + tangent * (s * 0.16) << end + radial * (s * 0.13) << end - radial * (s * 0.13);
			p->setBrush(c);
			p->drawPolygon(head);
			break;
		}
		case Glyph::Person:
		{
			// Kopf und Schultern.
			const QPointF head = P(0.5, 0.31);
			p->drawEllipse(head, s * 0.17, s * 0.17);
			const QPointF base = P(0.5, 0.94);
			const qreal r = 0.34 * s;
			QPainterPath body;
			body.moveTo(base + QPointF(-r, 0));
			body.arcTo(QRectF(base.x() - r, base.y() - r, 2 * r, 2 * r), 180, -180);
			body.closeSubpath();
			p->drawPath(body);
			break;
		}
		case Glyph::Topology:
		{
			// Ein Knoten oben, Linie zu zwei Knoten unten.
			p->drawRect(QRectF(P(0.36, 0.08), P(0.64, 0.30)));
			p->drawRect(QRectF(P(0.06, 0.70), P(0.34, 0.92)));
			p->drawRect(QRectF(P(0.66, 0.70), P(0.94, 0.92)));
			p->drawLine(P(0.5, 0.30), P(0.5, 0.50));
			p->drawLine(P(0.2, 0.50), P(0.8, 0.50));
			p->drawLine(P(0.2, 0.50), P(0.2, 0.70));
			p->drawLine(P(0.8, 0.50), P(0.8, 0.70));
			break;
		}
		case Glyph::Cable:
		{
			// Stecker mit zwei Kontakten, Kabel nach unten.
			p->drawRoundedRect(QRectF(P(0.26, 0.28), P(0.74, 0.62)), s * 0.06, s * 0.06);
			p->drawLine(P(0.38, 0.08), P(0.38, 0.28));
			p->drawLine(P(0.62, 0.08), P(0.62, 0.28));
			QPainterPath cable;
			cable.moveTo(P(0.5, 0.62));
			cable.cubicTo(P(0.5, 0.80), P(0.22, 0.78), P(0.22, 0.95));
			p->drawPath(cable);
			break;
		}
		case Glyph::Layers:
		{
			// Drei gestapelte Ebenen (VLANs).
			for (int i = 0; i < 3; ++i)
			{
				const qreal y = 0.24 + 0.22 * i;
				QPolygonF d;
				d << P(0.5, y - 0.14) << P(0.92, y) << P(0.5, y + 0.14) << P(0.08, y);
				if (i == 0)
					p->drawPolygon(d);
				else
				{
					p->drawLine(P(0.08, y), P(0.5, y + 0.14));
					p->drawLine(P(0.5, y + 0.14), P(0.92, y));
				}
			}
			break;
		}
		case Glyph::Route:
		{
			// Start, geschwungener Weg, Ziel.
			p->drawEllipse(P(0.2, 0.8), s * 0.09, s * 0.09);
			p->drawEllipse(P(0.8, 0.2), s * 0.09, s * 0.09);
			QPainterPath w;
			w.moveTo(P(0.29, 0.8));
			w.cubicTo(P(0.9, 0.8), P(0.1, 0.2), P(0.71, 0.2));
			p->drawPath(w);
			break;
		}
		case Glyph::Devices:
		{
			// Bildschirm mit Fuß und daneben ein Telefon.
			p->drawRect(QRectF(P(0.04, 0.16), P(0.66, 0.62)));
			p->drawLine(P(0.35, 0.62), P(0.35, 0.80));
			p->drawLine(P(0.20, 0.82), P(0.50, 0.82));
			p->drawRoundedRect(QRectF(P(0.72, 0.36), P(0.96, 0.86)), s * 0.04, s * 0.04);
			break;
		}
		case Glyph::Rows:
		{
			for (int i = 0; i < 3; ++i)
				p->drawRect(QRectF(P(0.10, 0.12 + 0.28 * i), P(0.90, 0.30 + 0.28 * i)));
			break;
		}
		case Glyph::Tag:
		{
			// Etikett mit Loch.
			QPolygonF t;
			t << P(0.08, 0.10) << P(0.50, 0.10) << P(0.92, 0.52) << P(0.52, 0.92) << P(0.08, 0.50);
			p->drawPolygon(t);
			p->drawEllipse(P(0.30, 0.31), s * 0.07, s * 0.07);
			break;
		}
		case Glyph::Fit:
		{
			// Vier Ecken nach außen.
			for (auto [cx, cy, dx, dy] : { std::array<qreal, 4>{ 0.1, 0.1, 1, 1 }, { 0.9, 0.1, -1, 1 }, { 0.1, 0.9, 1, -1 }, { 0.9, 0.9, -1, -1 } })
			{
				p->drawLine(P(cx, cy), P(cx + dx * 0.28, cy));
				p->drawLine(P(cx, cy), P(cx, cy + dy * 0.28));
			}
			p->drawRect(QRectF(P(0.34, 0.34), P(0.66, 0.66)));
			break;
		}
		case Glyph::ActualSize:
		{
			// Lupe mit „1:1“-Strichen.
			const QPointF m = P(0.42, 0.42);
			p->drawEllipse(m, s * 0.30, s * 0.30);
			p->drawLine(onCircle(m, 0.31, -45), P(0.90, 0.90));
			p->drawLine(P(0.32, 0.30), P(0.32, 0.54));
			p->drawLine(P(0.52, 0.30), P(0.52, 0.54));
			break;
		}
		case Glyph::Maximize:
		{
			// Fenster mit Pfeil nach rechts oben.
			p->drawRect(QRectF(P(0.1, 0.3), P(0.7, 0.9)));
			p->drawLine(P(0.45, 0.55), P(0.9, 0.1));
			p->drawLine(P(0.9, 0.1), P(0.62, 0.1));
			p->drawLine(P(0.9, 0.1), P(0.9, 0.38));
			break;
		}
		case Glyph::ExpandAll:
		{
			// Zwei Kästchen mit Plus.
			for (qreal y : { 0.08, 0.54 })
			{
				p->drawRect(QRectF(P(0.2, y), P(0.58, y + 0.38)));
				p->drawLine(P(0.29, y + 0.19), P(0.49, y + 0.19));
				p->drawLine(P(0.39, y + 0.09), P(0.39, y + 0.29));
			}
			p->drawLine(P(0.66, 0.27), P(0.9, 0.27));
			p->drawLine(P(0.66, 0.73), P(0.9, 0.73));
			break;
		}
		case Glyph::CollapseAll:
		{
			// Zwei Kästchen mit Minus.
			for (qreal y : { 0.08, 0.54 })
			{
				p->drawRect(QRectF(P(0.2, y), P(0.58, y + 0.38)));
				p->drawLine(P(0.29, y + 0.19), P(0.49, y + 0.19));
			}
			p->drawLine(P(0.66, 0.27), P(0.9, 0.27));
			p->drawLine(P(0.66, 0.73), P(0.9, 0.73));
			break;
		}
		case Glyph::Backup:
		{
			// Datenbank-Zylinder, daneben Pfeil nach unten.
			p->drawEllipse(QRectF(P(0.06, 0.08), P(0.62, 0.26)));
			p->drawLine(P(0.06, 0.17), P(0.06, 0.8));
			p->drawLine(P(0.62, 0.17), P(0.62, 0.8));
			p->drawArc(QRectF(P(0.06, 0.71), P(0.62, 0.89)), 180 * 16, 180 * 16);
			p->drawArc(QRectF(P(0.06, 0.4), P(0.62, 0.58)), 180 * 16, 180 * 16);
			p->drawLine(P(0.82, 0.3), P(0.82, 0.86));
			p->drawLine(P(0.82, 0.86), P(0.7, 0.72));
			p->drawLine(P(0.82, 0.86), P(0.94, 0.72));
			break;
		}
		case Glyph::Octets:
		{
			// „.2.1“: zwei Ziffern mit Punkten davor.
			QFont f = p->font();
			f.setPixelSize(qMax(7, int(s * 0.5)));
			f.setBold(true);
			p->setFont(f);
			p->drawText(QRectF(P(0.0, 0.0), P(1.0, 1.0)), Qt::AlignCenter, QStringLiteral(".2.1"));
			break;
		}
		case Glyph::Report:
		{
			// Blatt mit Zeilen, unten rechts ein Haken.
			p->drawRect(QRectF(P(0.16, 0.06), P(0.74, 0.94)));
			p->drawLine(P(0.28, 0.24), P(0.62, 0.24));
			p->drawLine(P(0.28, 0.40), P(0.62, 0.40));
			p->drawLine(P(0.28, 0.56), P(0.50, 0.56));
			p->drawLine(P(0.56, 0.76), P(0.68, 0.88));
			p->drawLine(P(0.68, 0.88), P(0.94, 0.58));
			break;
		}
		case Glyph::Print:
		{
			// Blatt oben, Gehäuse, ausgegebenes Blatt unten.
			p->drawRect(QRectF(P(0.28, 0.08), P(0.72, 0.36)));
			p->drawRoundedRect(QRectF(P(0.08, 0.36), P(0.92, 0.72)), 2, 2);
			p->drawRect(QRectF(P(0.28, 0.58), P(0.72, 0.92)));
			p->drawLine(P(0.36, 0.7), P(0.64, 0.7));
			p->drawLine(P(0.36, 0.8), P(0.64, 0.8));
			break;
		}
		case Glyph::Gear:
		{
			const QPointF m = P(0.5, 0.5);
			QPolygonF gear;
			for (int i = 0; i < 8; ++i)
			{
				const qreal t = i * 45.0;
				gear << onCircle(m, 0.31, t - 17) << onCircle(m, 0.45, t - 9) << onCircle(m, 0.45, t + 9) << onCircle(m, 0.31, t + 17);
			}
			p->drawPolygon(gear);
			p->drawEllipse(m, s * 0.12, s * 0.12);
			break;
		}
		}
		p->restore();
	}
	QPixmap pixmap(const QSize& size, QIcon::Mode mode, QIcon::State state) override { return scaledPixmap(size, mode, state, 1.0); }
	QPixmap scaledPixmap(const QSize& size, QIcon::Mode mode, QIcon::State state, qreal scale) override
	{
		QPixmap pm(size * scale);
		pm.setDevicePixelRatio(scale);
		pm.fill(Qt::transparent);
		QPainter p(&pm);
		paint(&p, QRect(QPoint(0, 0), size), mode, state);
		return pm;
	}
	QIconEngine* clone() const override { return new GlyphEngine(m_glyph); }

private:
	Glyph m_glyph;
};
} // namespace

QIcon glyphIcon(Glyph g)
{
	return QIcon(new GlyphEngine(g));
}

QIcon plugIcon(const QColor& c)
{
	QIcon icon;
	for (int px : { 16, 32, 64 })
	{
		QPixmap pm(px, px);
		pm.fill(Qt::transparent);
		QPainter p(&pm);
		p.setRenderHint(QPainter::Antialiasing);
		const qreal s = px;
		auto P = [s](qreal x, qreal y) { return QPointF(x * s, y * s); };
		QPen pen(c, qMax<qreal>(1.2, s / 9.0));
		pen.setCapStyle(Qt::RoundCap);
		pen.setJoinStyle(Qt::RoundJoin);
		p.setPen(pen);
		// Kontakte oben, Gehäuse, Kabel nach unten.
		p.drawLine(P(0.37, 0.06), P(0.37, 0.3));
		p.drawLine(P(0.63, 0.06), P(0.63, 0.3));
		p.setBrush(c);
		p.drawRoundedRect(QRectF(P(0.22, 0.3), P(0.78, 0.62)), s * 0.08, s * 0.08);
		p.setBrush(Qt::NoBrush);
		p.drawLine(P(0.5, 0.62), P(0.5, 0.94));
		p.end();
		icon.addPixmap(pm);
	}
	return icon;
}

QString poeStage(const QJsonObject& port)
{
	const double w = port.value(QStringLiteral("poe_w")).toDouble();
	const int cls = port.value(QStringLiteral("poe_class")).toInt(-1);
	if (port.value(QStringLiteral("poe_pp")).toBool() || cls >= 5 || w > 30.0)
		return QStringLiteral("++");
	if (cls == 4 || w > 15.4)
		return QStringLiteral("+");
	return QString();
}

void makeCopyable(QTableWidget* t, const std::function<QString(const char*)>& text, const std::function<void(QMenu*, int row)>& extra)
{
	t->setSelectionBehavior(QAbstractItemView::SelectRows);
	t->setSelectionMode(QAbstractItemView::ExtendedSelection);
	auto rows = [t](bool all) {
		QStringList head;
		for (int c = 0; c < t->columnCount(); ++c)
			head << (t->horizontalHeaderItem(c) ? t->horizontalHeaderItem(c)->text() : QString());
		QStringList out = { head.join(QLatin1Char('\t')) };
		for (int r = 0; r < t->rowCount(); ++r)
		{
			// Ausgefilterte Zeilen gehören nicht dazu.
			if (t->isRowHidden(r) || (!all && !t->selectionModel()->isRowSelected(r, QModelIndex())))
				continue;
			QStringList cells;
			for (int c = 0; c < t->columnCount(); ++c)
				cells << (t->item(r, c) ? t->item(r, c)->text() : QString());
			out << cells.join(QLatin1Char('\t'));
		}
		QGuiApplication::clipboard()->setText(out.join(QLatin1Char('\n')) + QLatin1Char('\n'));
	};
	auto* sc = new QShortcut(QKeySequence::Copy, t);
	sc->setContext(Qt::WidgetShortcut);
	QObject::connect(sc, &QShortcut::activated, t, [rows] { rows(false); });
	t->setContextMenuPolicy(Qt::CustomContextMenu);
	const QString one = text("table.copy"), all = text("table.copy.all");
	QObject::connect(t, &QWidget::customContextMenuRequested, t, [t, rows, one, all, extra](const QPoint& pos) {
		// Die Zeile unter dem Mauszeiger gilt, auch wenn sie noch nicht gewählt war.
		const int row = t->rowAt(pos.y());
		if (row >= 0 && !t->selectionModel()->isRowSelected(row, QModelIndex()))
			t->setCurrentCell(row, qMax(0, t->columnAt(pos.x())));
		QMenu menu(t);
		menu.addAction(one, [rows] { rows(false); })->setEnabled(t->selectionModel()->hasSelection());
		menu.addAction(all, [rows] { rows(true); });
		if (extra)
		{
			menu.addSeparator();
			extra(&menu, row);
		}
		menu.exec(t->viewport()->mapToGlobal(pos));
	});
}


ClickRow::ClickRow(QWidget* parent)
  : QFrame(parent)
{
	setObjectName(QStringLiteral("rjRow"));
	setCursor(Qt::PointingHandCursor);
	setFocusPolicy(Qt::StrongFocus);
	setAttribute(Qt::WA_Hover);
}

void ClickRow::setSelected(bool on)
{
	if (property("selected").toBool() == on)
		return;
	setProperty("selected", on);
	style()->unpolish(this);
	style()->polish(this);
}

void ClickRow::mouseReleaseEvent(QMouseEvent* e)
{
	if (e->button() == Qt::LeftButton && rect().contains(e->position().toPoint()))
		emit clicked();
	QFrame::mouseReleaseEvent(e);
}

void ClickRow::keyPressEvent(QKeyEvent* e)
{
	if (e->key() == Qt::Key_Return || e->key() == Qt::Key_Enter || e->key() == Qt::Key_Space)
	{
		emit clicked();
		return;
	}
	QFrame::keyPressEvent(e);
}

FlowBox::FlowBox(QWidget* parent, int spacing)
  : QWidget(parent)
  , m_flow(new FlowLayout(this, spacing))
{
}

void FlowBox::relayout()
{
	m_flow->invalidate();
	const int h = m_flow->heightForWidth(qMax(1, width()));
	setMinimumHeight(h);
	setMaximumHeight(h);
	m_flow->setGeometry(rect());
}

void FlowBox::resizeEvent(QResizeEvent* e)
{
	QWidget::resizeEvent(e);
	if (e->oldSize().width() != e->size().width())
		relayout();
}

FlowLayout::FlowLayout(QWidget* parent, int spacing)
  : QLayout(parent)
  , m_spacing(spacing)
{
	setContentsMargins(0, 0, 0, 0);
}

FlowLayout::~FlowLayout()
{
	while (QLayoutItem* i = takeAt(0))
		delete i;
}

void FlowLayout::addItem(QLayoutItem* item)
{
	m_items.append(item);
}

int FlowLayout::count() const
{
	return int(m_items.size());
}

QLayoutItem* FlowLayout::itemAt(int index) const
{
	return m_items.value(index);
}

QLayoutItem* FlowLayout::takeAt(int index)
{
	return index >= 0 && index < m_items.size() ? m_items.takeAt(index) : nullptr;
}

Qt::Orientations FlowLayout::expandingDirections() const
{
	return {};
}

bool FlowLayout::hasHeightForWidth() const
{
	return true;
}

int FlowLayout::heightForWidth(int width) const
{
	return doLayout(QRect(0, 0, width, 0), true);
}

QSize FlowLayout::minimumSize() const
{
	QSize s;
	for (QLayoutItem* i : m_items)
		s = s.expandedTo(i->minimumSize());
	return s;
}

QSize FlowLayout::sizeHint() const
{
	return minimumSize();
}

void FlowLayout::setGeometry(const QRect& rect)
{
	QLayout::setGeometry(rect);
	doLayout(rect, false);
}

int FlowLayout::doLayout(const QRect& rect, bool testOnly) const
{
	int x = rect.x(), y = rect.y(), line = 0;
	for (QLayoutItem* i : m_items)
	{
		if (i->widget() && i->widget()->isHidden())
			continue;
		const QSize s = i->sizeHint();
		if (x + s.width() > rect.right() + 1 && line > 0)
		{
			x = rect.x();
			y += line + m_spacing;
			line = 0;
		}
		if (!testOnly)
			i->setGeometry(QRect(QPoint(x, y), s));
		x += s.width() + m_spacing;
		line = qMax(line, s.height());
	}
	return y + line - rect.y();
}
} // namespace rjui
