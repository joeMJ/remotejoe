#include "cards.h"

#include <QtGui/QKeyEvent>
#include <QtGui/QMouseEvent>
#include <QtGui/QResizeEvent>
#include <QtWidgets/QLabel>
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
