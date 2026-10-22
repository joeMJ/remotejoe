#include "arubaaptab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtGui/QFontDatabase>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QScrollArea>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QVBoxLayout>

ArubaApTab::ArubaApTab(RjBackend* backend, const QJsonObject& subject, const QJsonObject& ap, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_subject(subject)
  , m_ap(ap)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	m_key = subject.value(QStringLiteral("id")).toString() + QLatin1Char('|') + ap.value(QStringLiteral("ip")).toString();
	setProperty("editor", true);
	auto* outer = new QVBoxLayout(this);
	outer->setContentsMargins(0, 0, 0, 0);
	auto* scroll = new QScrollArea(this);
	scroll->setWidgetResizable(true);
	scroll->setFrameShape(QFrame::NoFrame);
	auto* page = new QWidget(scroll);
	scroll->setWidget(page);
	outer->addWidget(scroll);
	auto* lay = new QVBoxLayout(page);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);
	const QFont mono = QFontDatabase::systemFont(QFontDatabase::FixedFont);

	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(page, &hin);
	auto* hh = new QHBoxLayout;
	auto* hv = new QVBoxLayout;
	auto* caption = new QLabel(ap.value(QStringLiteral("name")).toString(), head);
	QFont tf = mono;
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	caption->setFont(tf);
	caption->setTextInteractionFlags(Qt::TextSelectableByMouse);
	hv->addWidget(caption);
	m_sub = new QLabel(head);
	m_sub->setObjectName(QStringLiteral("rjMuted"));
	m_sub->setWordWrap(true);
	m_sub->setText(T("aruba.ap.sub").arg(ap.value(QStringLiteral("ip")).toString(), subject.value(QStringLiteral("vc_name")).toString()));
	hv->addWidget(m_sub);
	hh->addLayout(hv, 1);
	auto* ssh = new QPushButton(T("aruba.ap.ssh"), head);
	m_again = new QPushButton(T("aruba.ap.again"), head);
	auto* close = new QPushButton(T("mass.close"), head);
	hh->addWidget(ssh, 0, Qt::AlignTop);
	hh->addWidget(m_again, 0, Qt::AlignTop);
	hh->addWidget(close, 0, Qt::AlignTop);
	hin->addLayout(hh);
	m_state = new QLabel(head);
	m_state->setObjectName(QStringLiteral("rjMuted"));
	m_state->setWordWrap(true);
	hin->addWidget(m_state);
	lay->addWidget(head);

	QVBoxLayout* kin = nullptr;
	auto* kc = rjui::card(page, &kin);
	kin->addWidget(rjui::section(T("ov.inventory"), kc));
	auto* tiles = new QHBoxLayout;
	tiles->addWidget(rjui::tile(T("aruba.ap.serial"), &m_serial, kc));
	tiles->addWidget(rjui::tile(T("aruba.ap.mac"), &m_mac, kc));
	tiles->addWidget(rjui::tile(T("aruba.ap.uptime"), &m_uptime, kc));
	tiles->addWidget(rjui::tile(T("aruba.ap.system"), &m_system, kc));
	kin->addLayout(tiles);
	for (QLabel* l : { m_serial, m_mac })
	{
		QFont f = l->font();
		f.setFamily(mono.family());
		l->setFont(f);
		l->setTextInteractionFlags(Qt::TextSelectableByMouse);
	}
	m_serial->setText(ap.value(QStringLiteral("serial")).toString(QStringLiteral("–")));
	lay->addWidget(kc);

	auto table = [&](const char* title, QLabel** info, QTableWidget** t) {
		QVBoxLayout* in = nullptr;
		auto* c = rjui::card(page, &in);
		in->addWidget(rjui::section(T(title), c));
		*info = new QLabel(c);
		(*info)->setObjectName(QStringLiteral("rjMuted"));
		in->addWidget(*info);
		*t = new QTableWidget(c);
		(*t)->setEditTriggers(QAbstractItemView::NoEditTriggers);
		(*t)->setSelectionBehavior(QAbstractItemView::SelectRows);
		(*t)->verticalHeader()->hide();
		(*t)->horizontalHeader()->setStretchLastSection(true);
		(*t)->setMinimumHeight(140);
		in->addWidget(*t);
		lay->addWidget(c);
	};
	table("aruba.ap.networks", &m_netsInfo, &m_nets);
	table("aruba.ap.clients", &m_clientsInfo, &m_clients);
	lay->addStretch(1);

	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_again, &QPushButton::clicked, this, &ArubaApTab::query);
	connect(ssh, &QPushButton::clicked, this, [this] { emit connectWayRequested(m_subject, QStringLiteral("ssh:") + m_ap.value(QStringLiteral("ip")).toString()); });
	query();
}

QString ArubaApTab::title() const
{
	return m_backend->text(QStringLiteral("aruba.ap.tab")).arg(m_ap.value(QStringLiteral("name")).toString());
}

void ArubaApTab::query()
{
	QJsonObject cmd = m_subject;
	cmd.insert(QStringLiteral("op"), QStringLiteral("aruba_ap"));
	cmd.insert(QStringLiteral("ip"), m_ap.value(QStringLiteral("ip")));
	cmd.insert(QStringLiteral("ap_name"), m_ap.value(QStringLiteral("name")));
	const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(m_backend->text(QStringLiteral("aruba.error")).arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	m_again->setEnabled(false);
	m_state->setText(m_backend->text(QStringLiteral("aruba.ap.busy")));
}

void ArubaApTab::fillTable(QTableWidget* t, const QJsonObject& table)
{
	const QJsonArray cols = table.value(QStringLiteral("columns")).toArray();
	const QJsonArray rows = table.value(QStringLiteral("rows")).toArray();
	t->clear();
	t->setColumnCount(int(cols.size()));
	t->setRowCount(int(rows.size()));
	QStringList heads;
	for (const QJsonValue& c : cols)
		heads << c.toString();
	t->setHorizontalHeaderLabels(heads);
	for (int r = 0; r < rows.size(); ++r)
	{
		const QJsonArray row = rows[r].toArray();
		for (int c = 0; c < row.size() && c < cols.size(); ++c)
			t->setItem(r, c, new QTableWidgetItem(row[c].toString()));
	}
	t->resizeColumnsToContents();
}

void ArubaApTab::handleResult(const QJsonObject& r)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	m_again->setEnabled(true);
	if (r.value(QStringLiteral("error")).isString())
	{
		m_state->setText(T("aruba.error").arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	m_mac->setText(r.value(QStringLiteral("mac")).toString(QStringLiteral("–")));
	m_uptime->setText(r.value(QStringLiteral("uptime")).toString(QStringLiteral("–")));
	m_system->setText(r.value(QStringLiteral("system")).toString(QStringLiteral("–")));
	const QJsonObject nets = r.value(QStringLiteral("networks")).toObject();
	const QJsonObject clients = r.value(QStringLiteral("clients")).toObject();
	fillTable(m_nets, nets);
	fillTable(m_clients, clients);
	m_netsInfo->setText(T("aruba.ap.networks.count").arg(nets.value(QStringLiteral("rows")).toArray().size()));
	m_clientsInfo->setText(T("aruba.ap.clients.count").arg(clients.value(QStringLiteral("rows")).toArray().size()));
	QStringList state;
	state << T("aruba.at").arg(QDateTime::fromSecsSinceEpoch(r.value(QStringLiteral("at")).toInteger()).toString(QStringLiteral("dd.MM.yyyy HH:mm")));
	if (!r.value(QStringLiteral("complete")).toBool(true))
		state << T("aruba.ap.incomplete");
	if (r.value(QStringLiteral("note")).isString())
		state << r.value(QStringLiteral("note")).toString();
	m_state->setText(state.join(QStringLiteral(" · ")));
}
