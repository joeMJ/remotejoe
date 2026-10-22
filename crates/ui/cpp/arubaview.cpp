#include "arubaview.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonArray>
#include <QtGui/QFontDatabase>
#include <QtWidgets/QButtonGroup>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QStyle>
#include <QtWidgets/QVBoxLayout>

namespace
{
/// Kachel eines APs: Statuspunkt, Oktett (feste Breite), „|“, Name bzw. MAC, Anwenderzahl.
class ApTile : public rjui::ClickRow
{
public:
	ApTile(const QJsonObject& ap, QWidget* parent)
	  : rjui::ClickRow(parent)
	{
		setObjectName(QStringLiteral("rjApTile"));
		const bool online = ap.value(QStringLiteral("online")).toBool();
		const QString ip = ap.value(QStringLiteral("ip")).toString();
		const bool inside = ap.value(QStringLiteral("in_segment")).toBool(true);
		const QFont mono = QFontDatabase::systemFont(QFontDatabase::FixedFont);
		auto* h = new QHBoxLayout(this);
		h->setContentsMargins(8, 4, 8, 4);
		h->setSpacing(6);
		auto* dot = new QLabel(QStringLiteral("●"), this);
		dot->setProperty("rjDot", online ? QStringLiteral("ok") : ap.value(QStringLiteral("uncertain")).toBool() ? QStringLiteral("warn") : QStringLiteral("err"));
		h->addWidget(dot);
		auto* oct = new QLabel(ip.isEmpty() ? QStringLiteral("–") : inside ? QLatin1Char('.') + ip.section(QLatin1Char('.'), 3) : ip, this);
		oct->setFont(mono);
		oct->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
		// Vier Zeichen („.240“): die Trennstriche stehen untereinander; fremdes Segment zeigt die ganze Adresse.
		if (inside || ip.isEmpty())
			oct->setFixedWidth(QFontMetrics(mono).horizontalAdvance(QStringLiteral(".240")));
		else
			oct->setProperty("rjWarnText", true);
		h->addWidget(oct);
		auto* sep = new QLabel(QStringLiteral("|"), this);
		sep->setObjectName(QStringLiteral("rjMuted"));
		h->addWidget(sep);
		if (ap.value(QStringLiteral("conductor")).toBool())
			h->addWidget(new QLabel(QStringLiteral("★"), this));
		auto* name = new QLabel(ap.value(QStringLiteral("name")).toString(), this);
		name->setFont(mono);
		// Platz für eine ganze MAC (17 Zeichen) in der tatsächlichen Festbreitenschrift.
		name->setMinimumWidth(QFontMetrics(mono).horizontalAdvance(QString(17, QLatin1Char('0'))));
		h->addWidget(name, 1);
		if (ap.value(QStringLiteral("clients")).isDouble())
		{
			// Platz für dreistellige Zahlen, rechts abgesetzt vom Namen.
			auto* badge = rjui::pill(QString::number(ap.value(QStringLiteral("clients")).toInt()), QStringLiteral("accent"), this);
			badge->setAlignment(Qt::AlignCenter);
			badge->setMinimumWidth(badge->fontMetrics().horizontalAdvance(QStringLiteral("888")) + 12);
			h->addSpacing(4);
			h->addWidget(badge);
		}
		// Einheitliche Breite (Raster) aus dem Bedarf der Inhalte: Oktett, Stern, MAC, dreistellige Zahl.
		setFixedWidth(qMax(300, h->sizeHint().width() + (ap.value(QStringLiteral("conductor")).toBool() ? 0 : fontMetrics().horizontalAdvance(QStringLiteral("★ ")))
		                            + (ap.value(QStringLiteral("clients")).isDouble() ? 0 : fontMetrics().horizontalAdvance(QStringLiteral("888")) + 30)));
	}
};
} // namespace

ArubaClusterView::ArubaClusterView(RjBackend* backend, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	auto* v = new QVBoxLayout(this);
	v->setContentsMargins(0, 0, 0, 0);
	v->setSpacing(8);
	auto* hh = new QHBoxLayout;
	m_head = new QLabel(this);
	m_head->setObjectName(QStringLiteral("rjMuted"));
	m_head->setWordWrap(true);
	// Verwaltung als eigene Plakette: nicht verwaltet grün, AirWave orange, Central rot.
	m_managed = rjui::pill(QString(), QStringLiteral("off"), this);
	m_managed->hide();
	hh->addWidget(m_managed, 0, Qt::AlignVCenter);
	hh->addWidget(m_head, 1);
	v->addLayout(hh);
	// Werkzeuge: lesend (Analyse, Inventar) und schreibend (Neustart, Update; nur unverwaltet und mit Admin-Konto).
	auto* th = new QHBoxLayout;
	// Zuerst die Abfrage, dann die Werkzeuge, alle in einer Zeile.
	m_query = new QPushButton(rjui::glyphIcon(rjui::Glyph::Signal), T("aruba.query"), this);
	th->addWidget(m_query);
	for (const char* tool : { "analyse", "inventar", "reload", "update" })
	{
		const QString name = QString::fromLatin1(tool);
		const rjui::Glyph g = name == QLatin1String("analyse") ? rjui::Glyph::Search
		  : name == QLatin1String("inventar")                ? rjui::Glyph::Library
		  : name == QLatin1String("reload")                  ? rjui::Glyph::Refresh
		                                                     : rjui::Glyph::Gear;
		auto* b = new QPushButton(rjui::glyphIcon(g), T((QByteArray("aruba.") + tool).constData()), this);
		connect(b, &QPushButton::clicked, this, [this, name] { emit toolRequested(name); });
		th->addWidget(b);
		if (name == QLatin1String("reload"))
			m_reload = b;
		else if (name == QLatin1String("update"))
			m_update = b;
	}
	th->addStretch(1);
	v->addLayout(th);

	auto* tiles = new QHBoxLayout;
	tiles->addWidget(rjui::tile(T("aruba.total"), &m_total, this));
	tiles->addWidget(rjui::tile(T("aruba.online"), &m_online, this));
	tiles->addWidget(rjui::tile(T("aruba.offline"), &m_offline, this));
	tiles->addWidget(rjui::tile(T("aruba.clients"), &m_clients, this));
	v->addLayout(tiles);

	auto* fh = new QHBoxLayout;
	m_filter = new QButtonGroup(this);
	int id = 0;
	for (const char* k : { "aruba.filter.all", "aruba.filter.online", "aruba.filter.offline", "aruba.filter.segment" })
	{
		auto* b = new QPushButton(T(k), this);
		b->setCheckable(true);
		b->setChecked(id == 0);
		m_filter->addButton(b, id++);
		fh->addWidget(b);
	}
	fh->addStretch(1);
	// Logs am Controller: ganz oder gefiltert (show log … | include …), nur Anzeige. Rechts in der Filterzeile,
	// damit er nicht unter „Cluster abfragen“ steht.
	fh->addWidget(rjui::logMenuButton(T, this, [this](const QString& kind, bool filtered) { emit logRequested(kind, filtered); }));
	v->addLayout(fh);

	m_box = new rjui::FlowBox(this, 6);
	v->addWidget(m_box);
	m_foot = new QLabel(this);
	m_foot->setObjectName(QStringLiteral("rjMuted"));
	m_foot->setWordWrap(true);
	v->addWidget(m_foot);

	connect(m_query, &QPushButton::clicked, this, &ArubaClusterView::queryRequested);
	connect(m_filter, &QButtonGroup::idClicked, this, [this] { render(); });
	render();
}

void ArubaClusterView::setCluster(const QJsonObject& c)
{
	m_cluster = c;
	render();
}

void ArubaClusterView::setBusy(bool busy)
{
	m_query->setEnabled(!busy);
	if (busy)
		m_foot->setText(m_backend->text(QStringLiteral("aruba.busy")));
}

void ArubaClusterView::setError(const QString& text)
{
	m_query->setEnabled(true);
	m_foot->setText(m_backend->text(QStringLiteral("aruba.error")).arg(text));
}

void ArubaClusterView::render()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	while (QLayoutItem* it = m_box->flow()->takeAt(0))
	{
		if (QWidget* w = it->widget())
			w->deleteLater();
		delete it;
	}
	const QJsonObject& c = m_cluster;
	if (c.isEmpty())
	{
		m_managed->hide();
		m_head->setText(T("aruba.head.none"));
		for (QLabel* l : { m_total, m_online, m_offline, m_clients })
			l->setText(QStringLiteral("–"));
		m_foot->setText(T("aruba.hint"));
		for (QPushButton* b : { m_reload, m_update })
		{
			b->setEnabled(false);
			b->setToolTip(T("aruba.write.query.first"));
		}
		m_box->relayout();
		return;
	}
	m_head->setText(T("aruba.head").arg(c.value(QStringLiteral("vc")).toString(), c.value(QStringLiteral("segment")).toString(QStringLiteral("–"))));
	const QString managed = c.value(QStringLiteral("managed")).toString();
	m_managed->setVisible(!managed.isEmpty());
	if (!managed.isEmpty())
		rjui::setPill(m_managed, m_backend->text(QStringLiteral("aruba.managed.") + managed),
		              managed == QLatin1String("none") ? QStringLiteral("ok") : managed == QLatin1String("airwave") ? QStringLiteral("warn") : QStringLiteral("err"));
	const QJsonObject w = c.value(QStringLiteral("write")).toObject();
	for (QPushButton* b : { m_reload, m_update })
	{
		b->setEnabled(w.value(QStringLiteral("ok")).toBool());
		b->setToolTip(w.value(QStringLiteral("ok")).toBool() ? QString() : T("aruba.write.blocked").arg(w.value(QStringLiteral("reason")).toString(T("aruba.write.query.first"))));
	}
	m_total->setText(QString::number(c.value(QStringLiteral("total")).toInt()));
	m_online->setText(QString::number(c.value(QStringLiteral("online")).toInt()));
	m_offline->setText(c.value(QStringLiteral("allowed_known")).toBool() ? QString::number(c.value(QStringLiteral("offline")).toInt()) : QStringLiteral("?"));
	m_clients->setText(QString::number(c.value(QStringLiteral("clients")).toInt()));
	const int f = m_filter->checkedId();
	for (const QJsonValue& v : c.value(QStringLiteral("aps")).toArray())
	{
		const QJsonObject ap = v.toObject();
		const bool online = ap.value(QStringLiteral("online")).toBool();
		if ((f == 1 && !online) || (f == 2 && online) || (f == 3 && ap.value(QStringLiteral("in_segment")).toBool(true)))
			continue;
		auto* tile = new ApTile(ap, m_box);
		const QString ip = ap.value(QStringLiteral("ip")).toString();
		QString tip = ap.value(QStringLiteral("name")).toString() + QStringLiteral(" · ") + (ip.isEmpty() ? T("aruba.ip.unknown") : ip);
		if (ap.value(QStringLiteral("clients")).isDouble())
			tip += QStringLiteral(" · ") + T("aruba.tip.clients").arg(ap.value(QStringLiteral("clients")).toInt());
		if (ap.value(QStringLiteral("serial")).isString())
			tip += QStringLiteral(" · ") + ap.value(QStringLiteral("serial")).toString();
		if (ap.value(QStringLiteral("conductor")).toBool())
			tip += QStringLiteral(" · ") + T("aruba.tip.conductor").arg(c.value(QStringLiteral("vc")).toString());
		if (ap.value(QStringLiteral("uncertain")).toBool())
			tip += QStringLiteral(" · ") + T("aruba.tip.uncertain");
		tile->setToolTip(tip);
		if (online && !ip.isEmpty())
			connect(tile, &rjui::ClickRow::clicked, this, [this, ap] { emit apClicked(ap); });
		m_box->flow()->addWidget(tile);
		// Sofort sichtbar: das Layout überspringt noch verborgene Kacheln, Qt zeigt neue Kinder sonst erst später.
		tile->show();
	}
	m_box->relayout();
	QStringList foot;
	foot << T("aruba.at").arg(QDateTime::fromSecsSinceEpoch(c.value(QStringLiteral("at")).toInteger()).toString(QStringLiteral("dd.MM.yyyy HH:mm")));
	if (!c.value(QStringLiteral("allowed_known")).toBool())
		foot << T("aruba.offline.unknown");
	else if (c.value(QStringLiteral("offline_uncertain")).toBool())
		foot << T("aruba.offline.uncertain");
	if (c.value(QStringLiteral("note")).isString())
		foot << c.value(QStringLiteral("note")).toString();
	m_foot->setText(foot.join(QStringLiteral(" · ")));
	m_query->setEnabled(true);
}
