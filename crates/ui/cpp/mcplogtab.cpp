#include "mcplogtab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtCore/QTimer>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QVBoxLayout>

McpLogTab::McpLogTab(RjBackend* backend, QWidget* parent)
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
	auto* hh = new QHBoxLayout;
	auto* caption = new QLabel(T("mcplog.head"), head);
	QFont tf = caption->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	caption->setFont(tf);
	hh->addWidget(caption, 1);
	auto* close = new QPushButton(T("mass.close"), head);
	hh->addWidget(close);
	hin->addLayout(hh);
	auto* intro = new QLabel(T("mcplog.intro"), head);
	intro->setObjectName(QStringLiteral("rjMuted"));
	intro->setWordWrap(true);
	hin->addWidget(intro);
	lay->addWidget(head);
	QVBoxLayout* tin = nullptr;
	auto* tc = rjui::card(this, &tin);
	m_table = new QTableWidget(tc);
	m_table->setEditTriggers(QAbstractItemView::NoEditTriggers);
	m_table->verticalHeader()->hide();
	m_table->setColumnCount(4);
	m_table->setHorizontalHeaderLabels({ T("mcplog.col.at"), T("mcplog.col.tool"), T("mcplog.col.args"), T("mcplog.col.result") });
	m_table->horizontalHeader()->setDefaultAlignment(Qt::AlignLeft | Qt::AlignVCenter);
	m_table->horizontalHeader()->setStretchLastSection(true);
	rjui::makeCopyable(m_table, T);
	tin->addWidget(m_table, 1);
	lay->addWidget(tc, 1);
	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	m_timer = new QTimer(this);
	m_timer->setInterval(2000);
	connect(m_timer, &QTimer::timeout, this, &McpLogTab::refresh);
	m_timer->start();
	refresh();
}

void McpLogTab::refresh()
{
	const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QStringLiteral("{\"op\":\"mcp_log\"}")).toUtf8()).object();
	const QJsonArray log = r.value(QStringLiteral("log")).toArray();
	if (log.size() == m_count)
		return;
	m_count = int(log.size());
	m_table->setRowCount(m_count);
	for (int i = 0; i < m_count; ++i)
	{
		// Neueste oben.
		const QJsonObject e = log.at(m_count - 1 - i).toObject();
		const QStringList cells = { QDateTime::fromSecsSinceEpoch(e.value(QStringLiteral("at")).toInteger()).toString(QStringLiteral("dd.MM.yyyy HH:mm:ss")), e.value(QStringLiteral("tool")).toString(),
			                        QString::fromUtf8(QJsonDocument(e.value(QStringLiteral("args")).toObject()).toJson(QJsonDocument::Compact)), e.value(QStringLiteral("result")).toString() };
		for (int c = 0; c < cells.size(); ++c)
			m_table->setItem(i, c, new QTableWidgetItem(cells.at(c)));
	}
	m_table->resizeColumnsToContents();
}
