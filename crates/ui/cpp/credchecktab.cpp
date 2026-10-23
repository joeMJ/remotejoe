#include "credchecktab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtCore/QTimer>
#include <QtWidgets/QCheckBox>
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

/// Fehlerhafte Ergebnisse: zum Wiederholen und für den Filter „nur Fehler“.
bool isError(const QString& state)
{
	static const QStringList errors = { QStringLiteral("abgelehnt"), QStringLiteral("gesperrt"), QStringLiteral("abgelaufen"), QStringLiteral("nicht erreichbar"),
		                                QStringLiteral("zertifikat"), QStringLiteral("fehler"), QStringLiteral("nicht geprüft") };
	return errors.contains(state);
}
} // namespace

CredCheckTab::CredCheckTab(RjBackend* backend, const QStringList& ids, const QString& title, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_title(title)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setProperty("editor", true);
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);
	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(this, &hin);
	auto* hh = new QHBoxLayout;
	auto* hv = new QVBoxLayout;
	hv->setSpacing(2);
	auto* caption = new QLabel(T("credcheck.head").arg(title), head);
	QFont tf = caption->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	caption->setFont(tf);
	caption->setWordWrap(true);
	hv->addWidget(caption);
	m_state = new QLabel(head);
	m_state->setObjectName(QStringLiteral("rjMuted"));
	m_state->setWordWrap(true);
	hv->addWidget(m_state);
	hh->addLayout(hv, 1);
	m_cancel = new QPushButton(T("mass.cancel"), head);
	m_retry = new QPushButton(T("mass.retry"), head);
	auto* close = new QPushButton(T("mass.close"), head);
	for (QPushButton* b : { m_cancel, m_retry, close })
		hh->addWidget(b, 0, Qt::AlignTop);
	hin->addLayout(hh);
	auto* fh = new QHBoxLayout;
	m_onlyErrors = new QCheckBox(T("credcheck.only.errors"), head);
	m_edit = new QPushButton(T("credcheck.edit"), head);
	m_edit->setEnabled(false);
	fh->addWidget(m_onlyErrors);
	fh->addStretch(1);
	fh->addWidget(m_edit);
	hin->addLayout(fh);
	lay->addWidget(head);

	QVBoxLayout* lin = nullptr;
	auto* listCard = rjui::card(this, &lin);
	m_list = new QTreeWidget(listCard);
	m_list->setRootIsDecorated(false);
	m_list->setUniformRowHeights(true);
	m_list->setColumnCount(7);
	m_list->setHeaderLabels({ T("mass.col.target"), T("mass.col.path"), T("credcheck.col.way"), T("credcheck.col.user"), T("credcheck.col.origin"), T("credcheck.col.result"), T("credcheck.col.detail") });
	m_list->header()->setDefaultAlignment(Qt::AlignLeft | Qt::AlignVCenter);
	m_list->header()->setStretchLastSection(true);
	lin->addWidget(m_list, 1);
	auto* hint = new QLabel(T("credcheck.hint"), listCard);
	hint->setObjectName(QStringLiteral("rjMuted"));
	hint->setWordWrap(true);
	lin->addWidget(hint);
	lay->addWidget(listCard, 1);

	m_poll = new QTimer(this);
	m_poll->setInterval(700);
	connect(m_poll, &QTimer::timeout, this, &CredCheckTab::refresh);
	connect(m_cancel, &QPushButton::clicked, this, [this] { edit({ { QStringLiteral("op"), QStringLiteral("credcheck_cancel") } }); });
	connect(m_retry, &QPushButton::clicked, this, [this] { start(m_failed); });
	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_onlyErrors, &QCheckBox::toggled, this, &CredCheckTab::applyFilter);
	connect(m_list, &QTreeWidget::itemSelectionChanged, this, [this] { m_edit->setEnabled(m_list->currentItem() != nullptr); });
	connect(m_edit, &QPushButton::clicked, this, [this] {
		if (QTreeWidgetItem* it = m_list->currentItem())
			emit editTarget(it->data(0, Qt::UserRole).toString());
	});
	connect(m_list, &QTreeWidget::itemDoubleClicked, this, [this](QTreeWidgetItem* it) { emit showTarget(it->data(0, Qt::UserRole).toString()); });
	start(ids);
}

QString CredCheckTab::edit(const QJsonObject& cmd) const
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

void CredCheckTab::start(const QStringList& ids)
{
	const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("credcheck_start") }, { QStringLiteral("ids"), QJsonArray::fromStringList(ids) }, { QStringLiteral("title"), m_title } }));
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

void CredCheckTab::refresh()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("credcheck_rows") } }));
	const QJsonArray rows = r.value(QStringLiteral("rows")).toArray();
	if (m_list->topLevelItemCount() != rows.size())
	{
		m_list->clear();
		for (int i = 0; i < rows.size(); ++i)
			new QTreeWidgetItem(m_list);
	}
	int ok = 0, bad = 0, other = 0;
	m_failed.clear();
	for (int i = 0; i < rows.size(); ++i)
	{
		const QJsonObject o = rows.at(i).toObject();
		const QString state = o.value(QStringLiteral("state")).toString();
		const QString key = QStringLiteral("credcheck.state.") + QString(state).replace(QLatin1Char(' '), QLatin1Char('_'));
		const QString label = m_backend->text(key) == key ? state : m_backend->text(key);
		QColor color;
		if (state == QLatin1String("ok"))
		{
			++ok;
			color = QColor(0x1b, 0x8a, 0x2a);
		}
		else if (isError(state))
		{
			++bad;
			const QString id = o.value(QStringLiteral("id")).toString();
			if (!m_failed.contains(id))
				m_failed << id;
			color = QColor(0xc0, 0x30, 0x30);
		}
		else if (state != QStringLiteral("läuft") && state != QLatin1String("wartet"))
			++other;
		QTreeWidgetItem* it = m_list->topLevelItem(i);
		it->setData(0, Qt::UserRole, o.value(QStringLiteral("id")).toString());
		it->setData(0, Qt::UserRole + 1, state);
		const QStringList cells = { o.value(QStringLiteral("name")).toString(), o.value(QStringLiteral("path")).toString(),
			                        o.value(QStringLiteral("way")).toString() + QStringLiteral(" (") + o.value(QStringLiteral("protocol")).toString() + QLatin1Char(')'),
			                        o.value(QStringLiteral("user")).toString(), o.value(QStringLiteral("origin")).toString(), label, o.value(QStringLiteral("detail")).toString() };
		for (int c = 0; c < cells.size(); ++c)
		{
			it->setText(c, cells.at(c));
			it->setToolTip(c, cells.at(c));
		}
		it->setForeground(5, color.isValid() ? QBrush(color) : QBrush());
	}
	for (int c = 0; c < 6; ++c)
		m_list->resizeColumnToContents(c);
	applyFilter();
	const int done = r.value(QStringLiteral("done")).toInt();
	const int total = r.value(QStringLiteral("total")).toInt();
	const bool running = r.value(QStringLiteral("running")).toBool();
	m_state->setText(T(running ? "credcheck.progress" : "credcheck.summary").arg(done).arg(total).arg(ok).arg(bad).arg(other));
	m_cancel->setEnabled(running);
	m_retry->setEnabled(!running && !m_failed.isEmpty());
	m_retry->setText(m_failed.isEmpty() ? T("mass.retry") : T("mass.retry.n").arg(m_failed.size()));
	if (!running && m_running)
	{
		m_running = false;
		m_poll->stop();
	}
}

void CredCheckTab::applyFilter()
{
	for (int i = 0; i < m_list->topLevelItemCount(); ++i)
	{
		QTreeWidgetItem* it = m_list->topLevelItem(i);
		it->setHidden(m_onlyErrors->isChecked() && !isError(it->data(0, Qt::UserRole + 1).toString()));
	}
}
