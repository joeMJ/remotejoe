#include "arubaclientstab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QUuid>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QVBoxLayout>

ArubaClientsTab::ArubaClientsTab(RjBackend* backend, const QJsonObject& subject, const QString& where, QWidget* parent)
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
	m_reload = new QPushButton(T("aruba.log.reload"), head);
	auto* close = new QPushButton(T("mass.close"), head);
	hh->addWidget(m_reload, 0, Qt::AlignTop);
	hh->addWidget(close, 0, Qt::AlignTop);
	hin->addLayout(hh);
	auto* fh = new QHBoxLayout;
	fh->addWidget(new QLabel(T("aruba.users.search"), head));
	m_filter = new QLineEdit(head);
	m_filter->setPlaceholderText(T("aruba.users.search.ph"));
	m_filter->setClearButtonEnabled(true);
	fh->addWidget(m_filter, 1);
	m_count = new QLabel(head);
	m_count->setObjectName(QStringLiteral("rjMuted"));
	fh->addWidget(m_count);
	hin->addLayout(fh);
	lay->addWidget(head);

	QVBoxLayout* tin = nullptr;
	auto* tc = rjui::card(this, &tin);
	m_table = new QTableWidget(tc);
	m_table->setEditTriggers(QAbstractItemView::NoEditTriggers);
	m_table->verticalHeader()->hide();
	m_table->horizontalHeader()->setStretchLastSection(true);
	m_table->horizontalHeader()->setDefaultAlignment(Qt::AlignLeft | Qt::AlignVCenter);
	m_table->setSortingEnabled(true);
	rjui::makeCopyable(m_table, T);
	tin->addWidget(m_table, 1);
	lay->addWidget(tc, 1);

	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_reload, &QPushButton::clicked, this, &ArubaClientsTab::reload);
	connect(m_filter, &QLineEdit::textChanged, this, &ArubaClientsTab::applyFilter);
	connect(m_table, &QTableWidget::cellClicked, this, [this](int row, int col) {
		if (col != m_apCol || !m_table->item(row, col))
			return;
		const QJsonObject ap = m_aps.value(m_table->item(row, col)->text().toLower()).toObject();
		if (!ap.value(QStringLiteral("ip")).toString().isEmpty())
			emit apRequested(m_subject, ap);
	});
	reload();
	// Gleich lostippen können.
	m_filter->setFocus();
}

QString ArubaClientsTab::title() const
{
	return m_backend->text(QStringLiteral("aruba.users.tab")).arg(m_where);
}

void ArubaClientsTab::reload()
{
	QJsonObject cmd = m_subject;
	cmd.insert(QStringLiteral("op"), QStringLiteral("aruba_clients"));
	cmd.insert(QStringLiteral("tag"), m_tag);
	const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(m_backend->text(QStringLiteral("aruba.error")).arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	m_reload->setEnabled(false);
	m_state->setText(m_backend->text(QStringLiteral("aruba.users.busy")));
}

void ArubaClientsTab::handleResult(const QJsonObject& r)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	m_reload->setEnabled(true);
	if (r.value(QStringLiteral("error")).isString())
	{
		m_state->setText(T("aruba.error").arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	const QJsonArray cols = r.value(QStringLiteral("columns")).toArray();
	const QJsonArray rows = r.value(QStringLiteral("rows")).toArray();
	m_aps = r.value(QStringLiteral("aps")).toObject();
	m_apCol = r.value(QStringLiteral("ap_col")).isDouble() ? r.value(QStringLiteral("ap_col")).toInt() : -1;
	m_table->setSortingEnabled(false);
	m_table->clear();
	m_table->setColumnCount(int(cols.size()));
	m_table->setRowCount(int(rows.size()));
	QStringList heads;
	for (const QJsonValue& c : cols)
		heads << c.toString();
	m_table->setHorizontalHeaderLabels(heads);
	QFont link = m_table->font();
	link.setUnderline(true);
	for (int i = 0; i < rows.size(); ++i)
	{
		const QJsonArray row = rows[i].toArray();
		for (int c = 0; c < row.size() && c < cols.size(); ++c)
		{
			auto* item = new QTableWidgetItem(row[c].toString());
			// Der AP ist anklickbar, wenn er im Cluster bekannt ist.
			if (c == m_apCol && m_aps.contains(row[c].toString().toLower()))
			{
				item->setFont(link);
				item->setForeground(palette().color(QPalette::Link));
				item->setToolTip(T("aruba.users.ap.tip"));
			}
			m_table->setItem(i, c, item);
		}
	}
	m_table->setSortingEnabled(true);
	m_table->resizeColumnsToContents();
	QStringList state;
	state << QStringLiteral("%1 · show clients").arg(r.value(QStringLiteral("host")).toString());
	state << T("aruba.at").arg(QDateTime::fromSecsSinceEpoch(r.value(QStringLiteral("at")).toInteger()).toString(QStringLiteral("dd.MM.yyyy HH:mm:ss")));
	if (!r.value(QStringLiteral("complete")).toBool(true))
		state << T("aruba.log.incomplete");
	if (r.value(QStringLiteral("note")).isString())
		state << r.value(QStringLiteral("note")).toString();
	m_state->setText(state.join(QStringLiteral(" · ")));
	applyFilter();
}

void ArubaClientsTab::applyFilter()
{
	// Suchbegriff in irgendeiner Spalte (Anmeldename, IP, MAC …), ohne Groß-/Kleinschreibung.
	const QString f = m_filter->text().trimmed();
	int shown = 0;
	for (int r = 0; r < m_table->rowCount(); ++r)
	{
		bool hit = f.isEmpty();
		for (int c = 0; !hit && c < m_table->columnCount(); ++c)
			hit = m_table->item(r, c) && m_table->item(r, c)->text().contains(f, Qt::CaseInsensitive);
		m_table->setRowHidden(r, !hit);
		shown += hit ? 1 : 0;
	}
	m_count->setText(m_backend->text(QStringLiteral("aruba.users.count")).arg(shown).arg(m_table->rowCount()));
}
