#include "arubabreakouttab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QUuid>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QMenu>
#include <QtGui/QColor>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QStyle>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QVBoxLayout>

namespace
{
// Spalten der Treffer-Tabelle.
enum Col
{
	ColIp,
	ColVerdict,
	ColName,
	ColMac,
	ColVc,
	ColSeen,
	ColSystem,
	ColMs,
	ColCount
};
}

ArubaBreakoutTab::ArubaBreakoutTab(RjBackend* backend, const QJsonObject& subject, const QString& where, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_subject(subject)
  , m_where(where)
  , m_tag(QUuid::createUuid().toString(QUuid::WithoutBraces))
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
	m_state->setTextInteractionFlags(Qt::TextSelectableByMouse);
	hv->addWidget(m_state);
	hh->addLayout(hv, 1);
	m_again = new QPushButton(T("aruba.breakout.again"), head);
	auto* close = new QPushButton(T("mass.close"), head);
	hh->addWidget(m_again, 0, Qt::AlignTop);
	hh->addWidget(close, 0, Qt::AlignTop);
	hin->addLayout(hh);
	auto* hint = new QLabel(T("aruba.breakout.hint"), head);
	hint->setWordWrap(true);
	hin->addWidget(hint);
	m_offline = new QLabel(head);
	m_offline->setWordWrap(true);
	m_offline->setTextInteractionFlags(Qt::TextSelectableByMouse);
	hin->addWidget(m_offline);
	// Aktionen für den gewählten Treffer; „Alle erkennen“ für alle noch nicht erkannten.
	auto* ah = new QHBoxLayout;
	ah->addWidget(new QLabel(T("aruba.breakout.selected"), head));
	for (const char* key : { "identify", "ssh", "web" })
	{
		auto* b = new QPushButton(T((QByteArray("aruba.breakout.") + key).constData()), head);
		b->setProperty("action", QString::fromLatin1(key));
		connect(b, &QPushButton::clicked, this, [this, key] {
			const QString ip = currentIp();
			if (ip.isEmpty())
				return;
			if (qstrcmp(key, "identify") == 0)
				identify(ip);
			else
				emit connectWayRequested(m_subject, QString::fromLatin1(key) + QLatin1Char(':') + ip);
		});
		ah->addWidget(b);
		m_rowButtons << b;
	}
	m_all = new QPushButton(T("aruba.breakout.identify.all"), head);
	connect(m_all, &QPushButton::clicked, this, [this] {
		for (int r = 0; r < m_table->rowCount(); ++r)
			if (m_table->item(r, ColIp) && m_table->item(r, ColVerdict) && m_table->item(r, ColVerdict)->text().isEmpty())
				identify(m_table->item(r, ColIp)->text());
	});
	ah->addWidget(m_all);
	ah->addStretch(1);
	hin->addLayout(ah);
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
	for (const char* k : { "ip", "verdict", "name", "mac", "vc", "seen", "system", "ms" })
		heads << T((QByteArray("aruba.breakout.col.") + k).constData());
	m_table->setHorizontalHeaderLabels(heads);
	rjui::makeCopyable(m_table, T, [this](QMenu* menu, int row) {
		if (row < 0 || !m_table->item(row, ColIp))
			return;
		const QString ip = m_table->item(row, ColIp)->text();
		for (QPushButton* b : std::as_const(m_rowButtons))
		{
			const QString action = b->property("action").toString();
			QAction* a = menu->addAction(b->text(), this, [this, ip, action] {
				if (action == QLatin1String("identify"))
					identify(ip);
				else
					emit connectWayRequested(m_subject, action + QLatin1Char(':') + ip);
			});
			a->setEnabled(action != QLatin1String("identify") || !m_identifying.contains(ip));
		}
	});
	connect(m_table, &QTableWidget::currentCellChanged, this, &ArubaBreakoutTab::updateActions);
	connect(m_table, &QTableWidget::cellDoubleClicked, this, [this](int row, int) {
		if (m_table->item(row, ColIp))
			emit connectWayRequested(m_subject, QStringLiteral("ssh:") + m_table->item(row, ColIp)->text());
	});
	tin->addWidget(m_table, 1);
	lay->addWidget(tc, 1);

	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_again, &QPushButton::clicked, this, &ArubaBreakoutTab::search);
	updateActions();
	search();
}

QString ArubaBreakoutTab::title() const
{
	return m_backend->text(QStringLiteral("aruba.breakout.tab")).arg(m_where);
}

void ArubaBreakoutTab::search()
{
	QJsonObject cmd = m_subject;
	cmd.insert(QStringLiteral("op"), QStringLiteral("aruba_breakouts"));
	cmd.insert(QStringLiteral("tag"), m_tag);
	const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(m_backend->text(QStringLiteral("aruba.error")).arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	// Vorführung: fertiges Ergebnis sofort.
	if (r.value(QStringLiteral("demo")).isObject())
	{
		handleResult(r.value(QStringLiteral("demo")).toObject());
		return;
	}
	m_again->setEnabled(false);
	m_state->setText(m_backend->text(QStringLiteral("aruba.breakout.busy")));
}

void ArubaBreakoutTab::handleResult(const QJsonObject& r)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	m_again->setEnabled(true);
	if (r.value(QStringLiteral("error")).isString())
	{
		m_state->setText(T("aruba.error").arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	QStringList offline;
	for (const QJsonValue& m : r.value(QStringLiteral("offline")).toArray())
		offline << m.toString();
	m_offline->setText(offline.isEmpty() ? T("aruba.breakout.offline.none") : T("aruba.breakout.offline").arg(offline.size()).arg(offline.join(QStringLiteral(", "))));
	m_offline->setProperty("rjWarnText", !offline.isEmpty());
	m_offline->style()->unpolish(m_offline);
	m_offline->style()->polish(m_offline);
	const QJsonArray hits = r.value(QStringLiteral("hits")).toArray();
	m_identifying.clear();
	m_table->setSortingEnabled(false);
	m_table->setRowCount(int(hits.size()));
	for (int i = 0; i < hits.size(); ++i)
		setRow(i, hits[i].toObject());
	m_table->setSortingEnabled(true);
	m_table->resizeColumnsToContents();
	QStringList state;
	state << T("aruba.breakout.summary").arg(r.value(QStringLiteral("segment")).toString()).arg(r.value(QStringLiteral("scanned")).toInt()).arg(hits.size());
	state << T("aruba.at").arg(QDateTime::fromSecsSinceEpoch(r.value(QStringLiteral("at")).toInteger()).toString(QStringLiteral("dd.MM.yyyy HH:mm:ss")));
	if (!r.value(QStringLiteral("complete")).toBool(true))
		state << T("aruba.breakout.incomplete");
	m_state->setText(state.join(QStringLiteral(" · ")));
	updateActions();
}

void ArubaBreakoutTab::setRow(int row, const QJsonObject& hit)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	auto put = [&](int col, const QString& text) {
		auto* it = m_table->item(row, col);
		if (!it)
		{
			it = new QTableWidgetItem;
			m_table->setItem(row, col, it);
		}
		it->setText(text);
		return it;
	};
	put(ColIp, hit.value(QStringLiteral("ip")).toString());
	if (hit.contains(QStringLiteral("ms")))
		put(ColMs, QStringLiteral("%1 ms").arg(hit.value(QStringLiteral("ms")).toInt()));
	put(ColName, hit.value(QStringLiteral("name")).toString());
	put(ColMac, hit.value(QStringLiteral("mac")).toString());
	put(ColVc, hit.value(QStringLiteral("claimed_vc")).toString());
	put(ColSeen, hit.value(QStringLiteral("aps_seen")).isDouble() ? QString::number(hit.value(QStringLiteral("aps_seen")).toInt()) : QString());
	put(ColSystem, hit.value(QStringLiteral("system")).toString());
	QString verdict;
	bool alarm = false;
	if (hit.value(QStringLiteral("error")).isString())
		verdict = T("aruba.error").arg(hit.value(QStringLiteral("error")).toString());
	else if (hit.contains(QStringLiteral("verdict")))
	{
		const QString v = hit.value(QStringLiteral("verdict")).toString();
		alarm = v == QLatin1String("breakout") || v == QLatin1String("claims_vc");
		verdict = !alarm && !hit.value(QStringLiteral("aruba")).toBool(true) ? T("aruba.breakout.verdict.none") : m_backend->text(QStringLiteral("aruba.breakout.verdict.") + v);
	}
	auto* vi = put(ColVerdict, verdict);
	QFont f = m_table->font();
	f.setBold(alarm);
	vi->setFont(f);
	vi->setForeground(alarm ? QColor(0xc6, 0x28, 0x28) : palette().color(QPalette::Text));
}

void ArubaBreakoutTab::identify(const QString& ip)
{
	if (m_identifying.contains(ip))
		return;
	QJsonObject cmd = m_subject;
	cmd.insert(QStringLiteral("op"), QStringLiteral("aruba_breakout_identify"));
	cmd.insert(QStringLiteral("ip"), ip);
	cmd.insert(QStringLiteral("tag"), m_tag);
	const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
	for (int row = 0; row < m_table->rowCount(); ++row)
		if (m_table->item(row, ColIp) && m_table->item(row, ColIp)->text() == ip)
		{
			if (r.contains(QStringLiteral("error")))
				setRow(row, QJsonObject{ { QStringLiteral("ip"), ip }, { QStringLiteral("error"), r.value(QStringLiteral("error")).toString() } });
			else
			{
				m_identifying.insert(ip, true);
				m_table->item(row, ColVerdict)->setText(m_backend->text(QStringLiteral("aruba.breakout.identifying")));
			}
		}
	updateActions();
}

void ArubaBreakoutTab::handleIdentify(const QJsonObject& r)
{
	const QString ip = r.value(QStringLiteral("ip")).toString();
	m_identifying.remove(ip);
	m_table->setSortingEnabled(false);
	for (int row = 0; row < m_table->rowCount(); ++row)
		if (m_table->item(row, ColIp) && m_table->item(row, ColIp)->text() == ip)
		{
			QJsonObject hit = r;
			// Die Antwortzeit der Suche bleibt stehen.
			hit.remove(QStringLiteral("ms"));
			setRow(row, hit);
		}
	m_table->setSortingEnabled(true);
	m_table->resizeColumnsToContents();
	updateActions();
}

QString ArubaBreakoutTab::currentIp() const
{
	const int row = m_table->currentRow();
	return row >= 0 && m_table->item(row, ColIp) ? m_table->item(row, ColIp)->text() : QString();
}

void ArubaBreakoutTab::updateActions()
{
	const QString ip = currentIp();
	for (QPushButton* b : std::as_const(m_rowButtons))
		b->setEnabled(!ip.isEmpty() && (b->property("action").toString() != QLatin1String("identify") || !m_identifying.contains(ip)));
	m_all->setEnabled(m_table->rowCount() > 0);
}
