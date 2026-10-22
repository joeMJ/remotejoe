#include "cards.h"

#include <cmath>

#include <QtCore/QtMath>
#include <QtGui/QIconEngine>
#include <QtGui/QKeyEvent>
#include <QtGui/QMouseEvent>
#include <QtGui/QPainter>
#include <QtGui/QPainterPath>
#include <QtGui/QResizeEvent>
#include <QtWidgets/QApplication>
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

QPushButton* logMenuButton(const std::function<QString(const char*)>& text, QWidget* parent, const std::function<void(const QString& kind, bool filtered)>& pick)
{
	auto* b = new QPushButton(glyphIcon(Glyph::Note), text("aruba.log.menu"), parent);
	auto* menu = new QMenu(b);
	for (const char* key : { "system", "system.filtered", static_cast<const char*>(nullptr), "security", "security.filtered" })
	{
		if (!key)
		{
			menu->addSeparator();
			continue;
		}
		const QString k = QString::fromLatin1(key);
		menu->addAction(text((QByteArray("aruba.log.") + key).constData()), b, [pick, k] { pick(k.section(QLatin1Char('.'), 0, 0), k.endsWith(QLatin1String(".filtered"))); });
	}
	b->setMenu(menu);
	return b;
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
