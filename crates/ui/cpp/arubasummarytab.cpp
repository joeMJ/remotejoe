#include "arubasummarytab.h"

#include "arubaview.h"
#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtGui/QClipboard>
#include <QtGui/QColor>
#include <QtGui/QGuiApplication>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QMenu>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QVBoxLayout>

namespace
{
// Spalten der Übersicht.
enum Col
{
	ColName,
	ColVc,
	ColManaged,
	ColTotal,
	ColOnline,
	ColOffline,
	ColClients,
	ColOfflineAps,
	ColReboot,
	ColAt,
	ColState,
	ColCount
};

QJsonObject edit(RjBackend* backend, const QJsonObject& cmd)
{
	return QJsonDocument::fromJson(backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
}
} // namespace

ArubaSummaryTab::ArubaSummaryTab(RjBackend* backend, const QStringList& ids, const QString& where, bool query, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_ids(ids)
  , m_where(where)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setProperty("editor", true);
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);
	// Die Tabelle zuerst: die Knöpfe im Kopf beziehen sich auf sie (Platz im Layout weiter unten).
	m_table = new QTableWidget(this);
	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(this, &hin);
	auto* hh = new QHBoxLayout;
	auto* hv = new QVBoxLayout;
	auto* caption = new QLabel(title(), head);
	QFont tf = caption->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	caption->setFont(tf);
	hv->addWidget(caption);
	m_state = new QLabel(head);
	m_state->setObjectName(QStringLiteral("rjMuted"));
	m_state->setWordWrap(true);
	hv->addWidget(m_state);
	hh->addLayout(hv, 1);
	m_query = new QPushButton(rjui::glyphIcon(rjui::Glyph::Signal), T("aruba.summary.query"), head);
	m_mark = new QPushButton(T("aruba.offline.mark"), head);
	m_mark->setToolTip(T("aruba.summary.mark.tip"));
	auto* close = new QPushButton(T("mass.close"), head);
	for (QPushButton* b : { m_query, m_mark, close })
		hh->addWidget(b, 0, Qt::AlignTop);
	hin->addLayout(hh);
	lay->addWidget(head);

	QVBoxLayout* tin = nullptr;
	auto* tc = rjui::card(this, &tin);
	m_table->setParent(tc);
	m_table->setEditTriggers(QAbstractItemView::NoEditTriggers);
	m_table->verticalHeader()->hide();
	m_table->horizontalHeader()->setStretchLastSection(true);
	m_table->horizontalHeader()->setDefaultAlignment(Qt::AlignLeft | Qt::AlignVCenter);
	m_table->setColumnCount(ColCount);
	QStringList heads;
	for (const char* k : { "name", "vc", "managed", "total", "online", "offline", "clients", "offline_aps", "reboot", "at", "state" })
		heads << T((QByteArray("aruba.summary.col.") + k).constData());
	m_table->setHorizontalHeaderLabels(heads);
	m_table->setSortingEnabled(true);
	rjui::makeCopyable(m_table, T, [this, T](QMenu* menu, int row) {
		const QString id = row >= 0 && m_table->item(row, ColName) ? m_table->item(row, ColName)->data(Qt::UserRole).toString() : QString();
		const int r = rowOf(id);
		QAction* copy = menu->addAction(T("aruba.offline.copy"), this, [this, r] {
			QGuiApplication::clipboard()->setText(ArubaClusterView::offlineText(m_backend, m_rows[r].name, m_rows[r].cluster));
		});
		copy->setEnabled(r >= 0 && !m_rows[r].cluster.isEmpty());
		menu->addAction(T("aruba.offline.mark"), m_mark, &QPushButton::click);
		menu->addSeparator();
		menu->addAction(T("aruba.summary.open"), this, [this, id] {
			if (!id.isEmpty())
				emit openTarget(id);
		})->setEnabled(!id.isEmpty());
	});
	connect(m_table, &QTableWidget::cellDoubleClicked, this, [this](int row, int) {
		if (m_table->item(row, ColName))
			emit openTarget(m_table->item(row, ColName)->data(Qt::UserRole).toString());
	});
	tin->addWidget(m_table, 1);
	lay->addWidget(tc, 1);

	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_query, &QPushButton::clicked, this, &ArubaSummaryTab::queryAll);
	connect(m_mark, &QPushButton::clicked, this, [this] {
		// Markierte Zeilen, sonst alle.
		QStringList ids = selectedIds();
		if (ids.isEmpty())
			for (const Row& r : std::as_const(m_rows))
				ids << r.id;
		const QJsonObject r = edit(m_backend, { { QStringLiteral("op"), QStringLiteral("aruba_mark") }, { QStringLiteral("ids"), QJsonArray::fromStringList(ids) } });
		emit status(r.contains(QStringLiteral("error")) ? r.value(QStringLiteral("error")).toString()
		                                                : m_backend->text(QStringLiteral("aruba.summary.marked")).arg(r.value(QStringLiteral("marked")).toInt()));
		load();
	});
	const QJsonObject settings = edit(m_backend, { { QStringLiteral("op"), QStringLiteral("settings") } });
	m_parallel = qMax(1, settings.value(QStringLiteral("connect_all_parallel")).toInt(4));
	load();
	if (query)
		queryAll();
}

QString ArubaSummaryTab::title() const
{
	return m_backend->text(QStringLiteral("aruba.summary.tab")).arg(m_where);
}

void ArubaSummaryTab::load()
{
	// Gespeicherten Stand neu lesen; laufende Abfragen und Fehler bleiben stehen.
	const QJsonObject r = edit(m_backend, { { QStringLiteral("op"), QStringLiteral("aruba_summary") }, { QStringLiteral("ids"), QJsonArray::fromStringList(m_ids) } });
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(m_backend->text(QStringLiteral("aruba.error")).arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	QList<Row> rows;
	for (const QJsonValue& v : r.value(QStringLiteral("rows")).toArray())
	{
		const QJsonObject o = v.toObject();
		Row row;
		row.id = o.value(QStringLiteral("id")).toString();
		row.name = o.value(QStringLiteral("name")).toString();
		row.cluster = o.value(QStringLiteral("cluster")).toObject();
		if (const int old = rowOf(row.id); old >= 0)
		{
			row.error = m_rows[old].error;
			row.waiting = m_rows[old].waiting;
			row.running = m_rows[old].running;
		}
		rows << row;
	}
	m_rows = rows;
	m_table->setSortingEnabled(false);
	m_table->setRowCount(0);
	m_table->setRowCount(int(m_rows.size()));
	for (int i = 0; i < m_rows.size(); ++i)
		fillRow(i);
	m_table->setSortingEnabled(true);
	m_table->resizeColumnsToContents();
	updateState();
}

int ArubaSummaryTab::rowOf(const QString& id) const
{
	for (int i = 0; i < m_rows.size(); ++i)
		if (m_rows[i].id == id)
			return i;
	return -1;
}

QStringList ArubaSummaryTab::selectedIds() const
{
	QStringList ids;
	for (int r = 0; r < m_table->rowCount(); ++r)
		if (m_table->selectionModel()->isRowSelected(r, QModelIndex()) && m_table->item(r, ColName))
			ids << m_table->item(r, ColName)->data(Qt::UserRole).toString();
	return ids;
}

void ArubaSummaryTab::fillRow(int i)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const Row& r = m_rows[i];
	// Zeile der Tabelle zu diesem Eintrag (nach dem Sortieren nicht mehr gleich dem Index).
	int row = -1;
	for (int t = 0; t < m_table->rowCount(); ++t)
		if (m_table->item(t, ColName) && m_table->item(t, ColName)->data(Qt::UserRole).toString() == r.id)
			row = t;
	if (row < 0)
		row = i;
	const bool sorting = m_table->isSortingEnabled();
	m_table->setSortingEnabled(false);
	auto put = [&](int col, const QString& text) {
		auto* it = m_table->item(row, col);
		if (!it)
		{
			it = new QTableWidgetItem;
			m_table->setItem(row, col, it);
		}
		it->setText(text);
		it->setToolTip(QString());
		it->setBackground(QBrush());
		it->setForeground(palette().color(QPalette::Text));
		return it;
	};
	const QJsonObject& c = r.cluster;
	put(ColName, r.name)->setData(Qt::UserRole, r.id);
	put(ColVc, c.value(QStringLiteral("vc")).toString());
	const QString managed = c.value(QStringLiteral("managed")).toString();
	put(ColManaged, managed.isEmpty() ? QString() : m_backend->text(QStringLiteral("aruba.managed.") + managed));
	const bool known = !c.isEmpty();
	put(ColTotal, known ? QString::number(c.value(QStringLiteral("total")).toInt()) : QString());
	put(ColOnline, known ? QString::number(c.value(QStringLiteral("online")).toInt()) : QString());
	QString markTip;
	auto* off = put(ColOffline, known ? ArubaClusterView::offlineCount(m_backend, c, &markTip) : QString());
	off->setToolTip(markTip);
	const QStringList macs = known ? ArubaClusterView::offlineMacs(c) : QStringList();
	auto* list = put(ColOfflineAps, macs.join(QStringLiteral(", ")));
	list->setToolTip(macs.join(QLatin1Char('\n')));
	// Mindestens ein AP offline: Felder rot hinterlegen.
	if (known && c.value(QStringLiteral("allowed_known")).toBool() && c.value(QStringLiteral("offline")).toInt() > 0)
		for (auto* it : { off, list })
		{
			it->setBackground(QColor(217, 48, 37, 60));
			it->setForeground(QColor(179, 38, 30));
		}
	put(ColClients, known ? QString::number(c.value(QStringLiteral("clients")).toInt()) : QString());
	QString rebootTip;
	put(ColReboot, ArubaClusterView::rebootText(m_backend, c, &rebootTip, false))->setToolTip(rebootTip);
	put(ColAt, known ? QDateTime::fromSecsSinceEpoch(c.value(QStringLiteral("at")).toInteger()).toString(QStringLiteral("dd.MM.yyyy HH:mm")) : QString());
	QString state = r.running ? T("aruba.summary.running") : r.waiting ? T("aruba.summary.waiting") : !r.error.isEmpty() ? T("aruba.error").arg(r.error) : !known ? T("aruba.summary.never") : QString();
	auto* st = put(ColState, state);
	if (!r.error.isEmpty() && !r.running && !r.waiting)
		st->setForeground(QColor(179, 38, 30));
	m_table->setSortingEnabled(sorting);
}

void ArubaSummaryTab::updateState()
{
	int offline = 0, withOffline = 0, running = 0, waiting = 0, errors = 0;
	for (const Row& r : std::as_const(m_rows))
	{
		const int n = r.cluster.value(QStringLiteral("allowed_known")).toBool() ? r.cluster.value(QStringLiteral("offline")).toInt() : 0;
		offline += n;
		withOffline += n > 0 ? 1 : 0;
		running += r.running ? 1 : 0;
		waiting += r.waiting ? 1 : 0;
		errors += !r.error.isEmpty() && !r.running && !r.waiting ? 1 : 0;
	}
	QStringList s;
	s << m_backend->text(QStringLiteral("aruba.summary.count")).arg(m_rows.size()).arg(withOffline).arg(offline);
	if (running + waiting > 0)
		s << m_backend->text(QStringLiteral("aruba.summary.progress")).arg(running).arg(waiting);
	if (errors > 0)
		s << m_backend->text(QStringLiteral("aruba.summary.errors")).arg(errors);
	if (m_rows.isEmpty())
		s = QStringList{ m_backend->text(QStringLiteral("aruba.summary.empty")) };
	m_state->setText(s.join(QStringLiteral(" · ")));
	m_query->setEnabled(running + waiting == 0 && !m_rows.isEmpty());
}

void ArubaSummaryTab::queryAll()
{
	for (Row& r : m_rows)
	{
		r.waiting = true;
		r.running = false;
		r.error.clear();
	}
	for (int i = 0; i < m_rows.size(); ++i)
		fillRow(i);
	startNext();
}

void ArubaSummaryTab::startNext()
{
	int running = 0;
	for (const Row& r : std::as_const(m_rows))
		running += r.running ? 1 : 0;
	for (int i = 0; i < m_rows.size() && running < m_parallel; ++i)
	{
		Row& r = m_rows[i];
		if (!r.waiting)
			continue;
		r.waiting = false;
		const QJsonObject res = edit(m_backend, { { QStringLiteral("op"), QStringLiteral("aruba_cluster") }, { QStringLiteral("id"), r.id } });
		if (res.contains(QStringLiteral("error")))
			r.error = res.value(QStringLiteral("error")).toString();
		else
		{
			r.running = true;
			++running;
		}
		fillRow(i);
	}
	updateState();
}

void ArubaSummaryTab::handleResult(const QString& id, const QJsonObject& r)
{
	const int i = rowOf(id);
	if (i < 0)
		return;
	Row& row = m_rows[i];
	// Nur Ergebnisse eigener Abfragen zählen für den Fortschritt; der Stand wird in jedem Fall übernommen.
	const bool mine = row.running;
	row.running = false;
	if (r.value(QStringLiteral("error")).isString())
		row.error = r.value(QStringLiteral("error")).toString();
	else
	{
		row.error.clear();
		row.cluster = r;
	}
	fillRow(i);
	m_table->resizeColumnsToContents();
	if (mine)
		startNext();
	else
		updateState();
}
