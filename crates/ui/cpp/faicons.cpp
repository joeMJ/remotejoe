#include "faicons.h"

#include <QApplication>
#include <QDir>
#include <QFile>
#include <QStandardPaths>
#include <QHash>
#include <QIconEngine>
#include <QPainter>
#include <QPalette>
#include <QPixmap>
#include <QPixmapCache>
#include <QSvgRenderer>

namespace
{
// Rohtext der SVG-Dateien; einmal gelesen, danach aus dem Speicher.
const QByteArray& rawSvg(const QString& name)
{
	static QHash<QString, QByteArray> cache;
	auto it = cache.find(name);
	if (it != cache.end())
		return *it;
	QFile f(QStringLiteral(":/fa/") + name + QStringLiteral(".svg"));
	QByteArray data;
	if (f.open(QIODevice::ReadOnly))
		data = f.readAll();
	return *cache.insert(name, data);
}

QByteArray coloured(const QString& name, const QColor& c)
{
	QByteArray svg = rawSvg(name);
	svg.replace("<path ", "<path fill=\"" + c.name(QColor::HexRgb).toLatin1() + "\" ");
	if (c.alpha() < 255)
		svg.replace("<path ", "<path fill-opacity=\"" + QByteArray::number(c.alphaF(), 'f', 3) + "\" ");
	return svg;
}

QPixmap render(const QString& name, const QColor& c, const QSize& size, qreal dpr)
{
	const QString key = QStringLiteral("fa|%1|%2|%3x%4|%5").arg(name, c.name(QColor::HexArgb)).arg(size.width()).arg(size.height()).arg(dpr);
	QPixmap pm;
	if (QPixmapCache::find(key, &pm))
		return pm;
	pm = QPixmap(size * dpr);
	pm.setDevicePixelRatio(dpr);
	pm.fill(Qt::transparent);
	{
		QPainter p(&pm);
		faPaint(&p, QRectF(QPointF(0, 0), QSizeF(size)), name, c);
	}
	QPixmapCache::insert(key, pm);
	return pm;
}

class FaEngine : public QIconEngine
{
public:
	FaEngine(const QString& name, const QColor& color, const QColor& selected)
	  : m_name(name)
	  , m_color(color)
	  , m_selected(selected)
	{
	}
	QColor colorFor(QIcon::Mode mode) const
	{
		const QPalette pal = QApplication::palette();
		if (mode == QIcon::Disabled)
			return pal.color(QPalette::Disabled, QPalette::WindowText);
		if (mode == QIcon::Selected)
			return m_selected.isValid() ? m_selected : (m_color.isValid() ? m_color : pal.color(QPalette::HighlightedText));
		return m_color.isValid() ? m_color : faMutedColor();
	}
	void paint(QPainter* p, const QRect& rect, QIcon::Mode mode, QIcon::State) override
	{
		faPaint(p, QRectF(rect), m_name, colorFor(mode));
	}
	QPixmap pixmap(const QSize& size, QIcon::Mode mode, QIcon::State state) override { return scaledPixmap(size, mode, state, 1.0); }
	QPixmap scaledPixmap(const QSize& size, QIcon::Mode mode, QIcon::State, qreal scale) override
	{
		return render(m_name, colorFor(mode), size, scale);
	}
	QIconEngine* clone() const override { return new FaEngine(m_name, m_color, m_selected); }

private:
	QString m_name;
	QColor m_color;
	QColor m_selected;
};
} // namespace

QColor faMutedColor()
{
	const QPalette pal = QApplication::palette();
	const QColor t = pal.color(QPalette::Active, QPalette::WindowText);
	const QColor b = pal.color(QPalette::Active, QPalette::Window);
	// 72 % Text, 28 % Hintergrund: sichtbar grau, aber in Hell und Dunkel gut erkennbar.
	return QColor::fromRgbF(t.redF() * 0.72 + b.redF() * 0.28, t.greenF() * 0.72 + b.greenF() * 0.28, t.blueF() * 0.72 + b.blueF() * 0.28);
}

bool faHas(const QString& name)
{
	return !rawSvg(name).isEmpty();
}

void faPaint(QPainter* p, const QRectF& rect, const QString& name, const QColor& color)
{
	// Ein Renderer je Symbol und Farbe (das SVG wird nur einmal gelesen).
	static QHash<QString, QSvgRenderer*> renderers;
	const QString key = name + QLatin1Char('|') + color.name(QColor::HexArgb);
	QSvgRenderer*& rp = renderers[key];
	if (!rp)
		rp = new QSvgRenderer(coloured(name, color));
	QSvgRenderer& r = *rp;
	if (!r.isValid())
		return;
	// Seitenverhältnis des Symbols behalten, mittig ins Feld setzen.
	const QSizeF vb = r.viewBoxF().size();
	if (vb.isEmpty())
		return;
	const qreal s = qMin(rect.width() / vb.width(), rect.height() / vb.height());
	const QSizeF sz(vb.width() * s, vb.height() * s);
	const QRectF target(rect.x() + (rect.width() - sz.width()) / 2.0, rect.y() + (rect.height() - sz.height()) / 2.0, sz.width(), sz.height());
	p->save();
	p->setRenderHint(QPainter::Antialiasing);
	r.render(p, target);
	p->restore();
}

QIcon faIcon(const QString& name)
{
	return QIcon(new FaEngine(name, QColor(), QColor()));
}

QIcon faIcon(const QString& name, const QColor& color, const QColor& selected)
{
	return QIcon(new FaEngine(name, color, selected));
}

QString faFile(const QString& name, const QColor& color)
{
	static QString dir = [] {
		QString d = QStandardPaths::writableLocation(QStandardPaths::CacheLocation);
		if (d.isEmpty())
			d = QDir::tempPath() + QStringLiteral("/remotejoe");
		d += QStringLiteral("/symbole");
		QDir().mkpath(d);
		return d;
	}();
	const QString path = QStringLiteral("%1/%2-%3.svg").arg(dir, name, color.name(QColor::HexRgb).mid(1));
	if (QFile::exists(path))
		return path;
	QFile f(path);
	if (!f.open(QIODevice::WriteOnly))
		return QString();
	f.write(coloured(name, color));
	return path;
}
