#include "masstab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtCore/QTimer>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTreeWidget>
#include <QtWidgets/QVBoxLayout>

namespace
{
QJsonObject parseObj(const QString& s)
{
	return QJsonDocument::fromJson(s.toUtf8()).object();
}
} // namespace

MassInventoryTab::MassInventoryTab(RjBackend* backend, const QStringList& ids, const QString& title, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_title(title)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setObjectName(QStringLiteral("massInventoryTab"));
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);

	// Kopfkarte: was abgefragt wird, Fortschritt, Knöpfe.
	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(this, &hin);
	auto* hh = new QHBoxLayout;
	auto* hv = new QVBoxLayout;
	hv->setSpacing(2);
	m_head = new QLabel(T("mass.head").arg(title), head);
	QFont tf = m_head->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	m_head->setFont(tf);
	hv->addWidget(m_head);
	m_state = new QLabel(head);
	m_state->setObjectName(QStringLiteral("rjMuted"));
	m_state->setWordWrap(true);
	hv->addWidget(m_state);
	hh->addLayout(hv, 1);
	m_cancel = new QPushButton(T("mass.cancel"), head);
	m_retry = new QPushButton(T("mass.retry"), head);
	auto* close = new QPushButton(T("mass.close"), head);
	hh->addWidget(m_cancel, 0, Qt::AlignTop);
	hh->addWidget(m_retry, 0, Qt::AlignTop);
	hh->addWidget(close, 0, Qt::AlignTop);
	hin->addLayout(hh);
	lay->addWidget(head);

	// Karte mit den Zielen.
	QVBoxLayout* lin = nullptr;
	auto* listCard = rjui::card(this, &lin);
	lin->addWidget(rjui::section(T("mass.targets"), listCard));
	m_list = new QTreeWidget(listCard);
	m_list->setRootIsDecorated(false);
	m_list->setUniformRowHeights(true);
	m_list->setColumnCount(4);
	m_list->setHeaderLabels({ T("mass.col.target"), T("mass.col.path"), T("mass.col.host"), T("mass.col.result") });
	m_list->header()->setSectionResizeMode(0, QHeaderView::ResizeToContents);
	m_list->header()->setSectionResizeMode(1, QHeaderView::Interactive);
	m_list->header()->setSectionResizeMode(2, QHeaderView::ResizeToContents);
	m_list->header()->setSectionResizeMode(3, QHeaderView::Stretch);
	m_list->setColumnWidth(1, 260);
	lin->addWidget(m_list, 1);
	auto* hint = new QLabel(T("mass.hint"), listCard);
	hint->setObjectName(QStringLiteral("rjMuted"));
	hint->setWordWrap(true);
	lin->addWidget(hint);
	lay->addWidget(listCard, 1);

	m_poll = new QTimer(this);
	m_poll->setInterval(700);
	connect(m_poll, &QTimer::timeout, this, &MassInventoryTab::refresh);
	connect(m_cancel, &QPushButton::clicked, this, [this] { edit({ { QStringLiteral("op"), QStringLiteral("mass_cancel") } }); });
	connect(m_retry, &QPushButton::clicked, this, [this] { start(m_failed); });
	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_list, &QTreeWidget::itemDoubleClicked, this, [this](QTreeWidgetItem* it) { emit showTarget(it->data(0, Qt::UserRole).toString()); });
	start(ids);
}

QString MassInventoryTab::edit(const QJsonObject& cmd) const
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

void MassInventoryTab::start(const QStringList& ids)
{
	const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("mass_start") }, { QStringLiteral("ids"), QJsonArray::fromStringList(ids) }, { QStringLiteral("title"), m_title } }));
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(r.value(QStringLiteral("error")).toString());
		m_cancel->setEnabled(false);
		m_retry->setEnabled(!m_failed.isEmpty());
		return;
	}
	m_running = true;
	m_cancel->setEnabled(true);
	m_retry->setEnabled(false);
	refresh();
	m_poll->start();
}

void MassInventoryTab::refresh()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("mass_rows") } }));
	const QJsonArray rows = r.value(QStringLiteral("rows")).toArray();
	if (m_list->topLevelItemCount() != rows.size())
	{
		m_list->clear();
		for (int i = 0; i < rows.size(); ++i)
			new QTreeWidgetItem(m_list);
	}
	int found = 0, aruba = 0, failed = 0, skipped = 0;
	m_failed.clear();
	for (int i = 0; i < rows.size(); ++i)
	{
		const QJsonObject o = rows.at(i).toObject();
		const QString state = o.value(QStringLiteral("state")).toString();
		const QString detail = o.value(QStringLiteral("detail")).toString();
		QString result;
		QColor color;
		if (state == QLatin1String("erkannt"))
		{
			++found;
			if (o.value(QStringLiteral("aruba")).toBool())
				++aruba;
			result = detail;
			color = QColor(0x1b, 0x8a, 0x2a);
		}
		else if (state == QLatin1String("fehler"))
		{
			++failed;
			m_failed << o.value(QStringLiteral("id")).toString();
			result = T("mass.state.error").arg(detail);
			color = QColor(0xc0, 0x30, 0x30);
		}
		else if (state == QStringLiteral("übersprungen"))
		{
			++skipped;
			result = T("mass.state.skipped").arg(detail);
		}
		else if (state == QStringLiteral("läuft"))
			result = T("mass.state.running");
		else if (state == QLatin1String("abgebrochen"))
			result = T("mass.state.cancelled");
		else
			result = T("mass.state.waiting");
		QTreeWidgetItem* it = m_list->topLevelItem(i);
		it->setData(0, Qt::UserRole, o.value(QStringLiteral("id")).toString());
		const QStringList cells = { o.value(QStringLiteral("name")).toString(), o.value(QStringLiteral("path")).toString(), o.value(QStringLiteral("host")).toString(), result };
		for (int c = 0; c < cells.size(); ++c)
		{
			it->setText(c, cells.at(c));
			it->setToolTip(c, cells.at(c));
		}
		it->setForeground(3, color.isValid() ? QBrush(color) : QBrush());
	}
	const int done = r.value(QStringLiteral("done")).toInt();
	const int total = r.value(QStringLiteral("total")).toInt();
	const bool running = r.value(QStringLiteral("running")).toBool();
	m_state->setText(T(running ? "mass.progress" : "mass.summary").arg(done).arg(total).arg(found).arg(aruba).arg(failed).arg(skipped));
	m_cancel->setEnabled(running);
	m_retry->setEnabled(!running && !m_failed.isEmpty());
	m_retry->setText(m_failed.isEmpty() ? T("mass.retry") : T("mass.retry.n").arg(m_failed.size()));
	if (!running && m_running)
	{
		m_running = false;
		m_poll->stop();
		emit finished();
	}
}
