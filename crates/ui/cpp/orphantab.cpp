#include "orphantab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QVBoxLayout>

OrphanTab::OrphanTab(RjBackend* backend, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setProperty("editor", true);
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);
	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(this, &hin);
	auto* caption = new QLabel(T("orphans.head"), head);
	QFont tf = caption->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	caption->setFont(tf);
	hin->addWidget(caption);
	auto* intro = new QLabel(T("orphans.intro"), head);
	intro->setObjectName(QStringLiteral("rjMuted"));
	intro->setWordWrap(true);
	hin->addWidget(intro);
	m_state = new QLabel(head);
	m_state->setWordWrap(true);
	hin->addWidget(m_state);
	auto* bh = new QHBoxLayout;
	auto* all = new QPushButton(T("orphans.all"), head);
	m_recycle = new QPushButton(T("orphans.recycle"), head);
	m_undo = new QPushButton(T("orphans.undo"), head);
	auto* again = new QPushButton(T("orphans.again"), head);
	auto* close = new QPushButton(T("mass.close"), head);
	m_undo->setEnabled(false);
	for (QPushButton* b : { all, m_recycle, m_undo, again })
		bh->addWidget(b);
	bh->addStretch(1);
	bh->addWidget(close);
	hin->addLayout(bh);
	lay->addWidget(head);

	QVBoxLayout* tin = nullptr;
	auto* tc = rjui::card(this, &tin);
	m_table = new QTableWidget(tc);
	m_table->setEditTriggers(QAbstractItemView::NoEditTriggers);
	m_table->verticalHeader()->hide();
	m_table->horizontalHeader()->setStretchLastSection(true);
	m_table->horizontalHeader()->setDefaultAlignment(Qt::AlignLeft | Qt::AlignVCenter);
	m_table->setColumnCount(5);
	m_table->setHorizontalHeaderLabels({ T("orphans.col.title"), T("orphans.col.user"), T("orphans.col.path"), T("orphans.col.created"), T("orphans.col.modified") });
	rjui::makeCopyable(m_table, T);
	tin->addWidget(m_table, 1);
	lay->addWidget(tc, 1);

	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(again, &QPushButton::clicked, this, &OrphanTab::load);
	connect(all, &QPushButton::clicked, m_table, &QTableWidget::selectAll);
	connect(m_table, &QTableWidget::itemSelectionChanged, this, [this] { m_recycle->setEnabled(m_table->selectionModel()->hasSelection()); });
	connect(m_recycle, &QPushButton::clicked, this, [this] {
		QJsonArray entries;
		for (const QModelIndex& r : m_table->selectionModel()->selectedRows())
			entries.append(m_table->item(r.row(), 0)->data(Qt::UserRole).toString());
		if (!entries.isEmpty())
			move(true, entries);
	});
	connect(m_undo, &QPushButton::clicked, this, [this] {
		if (!m_last.isEmpty())
			move(false, m_last);
	});
	load();
}

QJsonObject OrphanTab::call(const QJsonObject& cmd) const
{
	return QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
}

void OrphanTab::load()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("orphans_list") } });
	const QJsonArray list = r.value(QStringLiteral("entries")).toArray();
	auto when = [](const QJsonValue& v) { return v.isDouble() ? QDateTime::fromSecsSinceEpoch(qint64(v.toDouble())).toString(QStringLiteral("dd.MM.yyyy HH:mm")) : QString(); };
	m_table->setSortingEnabled(false);
	m_table->clearContents();
	m_table->setRowCount(int(list.size()));
	for (int i = 0; i < list.size(); ++i)
	{
		const QJsonObject e = list[i].toObject();
		auto* title = new QTableWidgetItem(e.value(QStringLiteral("title")).toString());
		title->setData(Qt::UserRole, e.value(QStringLiteral("entry")).toString());
		m_table->setItem(i, 0, title);
		m_table->setItem(i, 1, new QTableWidgetItem(e.value(QStringLiteral("username")).toString()));
		m_table->setItem(i, 2, new QTableWidgetItem(e.value(QStringLiteral("path")).toString()));
		m_table->setItem(i, 3, new QTableWidgetItem(when(e.value(QStringLiteral("created")))));
		m_table->setItem(i, 4, new QTableWidgetItem(when(e.value(QStringLiteral("modified")))));
	}
	m_table->setSortingEnabled(true);
	m_table->resizeColumnsToContents();
	m_recycle->setEnabled(false);
	m_state->setText(r.value(QStringLiteral("locked")).toBool() ? T("orphans.locked")
	                 : list.isEmpty()                         ? T("orphans.none")
	                                                          : T("orphans.found").arg(list.size()));
}

void OrphanTab::move(bool recycle, const QJsonArray& entries)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const QJsonObject r = call({ { QStringLiteral("op"), recycle ? QStringLiteral("orphans_recycle") : QStringLiteral("orphans_restore") }, { QStringLiteral("entries"), entries } });
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(r.value(QStringLiteral("error")).toString());
		return;
	}
	const QJsonArray moved = r.value(QStringLiteral("moved")).toArray();
	m_last = recycle ? moved : QJsonArray();
	m_undo->setEnabled(!m_last.isEmpty());
	load();
	m_state->setText(T(recycle ? "orphans.recycled" : "orphans.restored").arg(moved.size()));
	emit status(m_state->text());
}
