#include "folderoverviewtab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtGui/QFontDatabase>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QVBoxLayout>

namespace
{
// Spalten: Typ, Name, Adresse, dann die Inventarfelder (Schlüssel aus `inventory_many`).
const char* const kFields[] = { "version", "system", "vcpu", "memory", "uptime", "serial", "mac" };
constexpr int kFirstField = 3;
constexpr int kAtColumn = kFirstField + 7;
constexpr int RoleItemId = Qt::UserRole + 1;
constexpr int RoleItemFolder = Qt::UserRole + 2;
} // namespace

FolderOverviewTab::FolderOverviewTab(RjBackend* backend, QWidget* parent)
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
	m_caption = new QLabel(head);
	QFont tf = m_caption->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	m_caption->setFont(tf);
	m_caption->setWordWrap(true);
	hin->addWidget(m_caption);
	m_sub = new QLabel(head);
	m_sub->setObjectName(QStringLiteral("rjMuted"));
	m_sub->setWordWrap(true);
	hin->addWidget(m_sub);
	auto* fh = new QHBoxLayout;
	fh->addWidget(new QLabel(T("folder.search"), head));
	m_filter = new QLineEdit(head);
	m_filter->setPlaceholderText(T("folder.search.ph"));
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
	QStringList heads = { T("folder.col.type"), T("folder.col.name"), T("folder.col.host") };
	for (const char* f : kFields)
		heads << m_backend->text(QStringLiteral("folder.col.") + QString::fromLatin1(f));
	heads << T("folder.col.at");
	m_table->setColumnCount(int(heads.size()));
	m_table->setHorizontalHeaderLabels(heads);
	rjui::makeCopyable(m_table, T);
	tin->addWidget(m_table, 1);
	lay->addWidget(tc, 1);

	connect(m_filter, &QLineEdit::textChanged, this, &FolderOverviewTab::applyFilter);
	connect(m_table, &QTableWidget::cellClicked, this, [this](int row, int) {
		const QTableWidgetItem* it = m_table->item(row, 0);
		if (!it)
			return;
		const QString id = it->data(RoleItemId).toString();
		if (it->data(RoleItemFolder).toBool())
			emit folderClicked(id);
		else
			emit targetClicked(id, it->icon());
	});
	connect(m_table, &QTableWidget::cellDoubleClicked, this, [this](int row, int) {
		const QTableWidgetItem* it = m_table->item(row, 0);
		if (it && !it->data(RoleItemFolder).toBool())
			emit targetActivated(it->data(RoleItemId).toString());
	});
}

QString FolderOverviewTab::title() const
{
	return m_backend->text(QStringLiteral("folder.tab")).arg(m_name);
}

void FolderOverviewTab::showFolder(const QString& id, const QString& name, const QString& path, const QList<Item>& items)
{
	m_id = id;
	m_name = name;
	m_caption->setText(name);
	m_sub->setText(path);
	// Ziele zuerst (Baumreihenfolge), Unterordner am Ende.
	QList<Item> ordered;
	for (const Item& i : items)
		if (!i.folder)
			ordered << i;
	for (const Item& i : items)
		if (i.folder)
			ordered << i;
	QJsonArray ids;
	for (const Item& i : ordered)
		if (!i.folder)
			ids.append(i.id);
	const QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("inventory_many") }, { QStringLiteral("ids"), ids } };
	const QJsonObject inv = QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();

	QFont strong = QFontDatabase::systemFont(QFontDatabase::FixedFont);
	strong.setBold(true);
	m_table->setSortingEnabled(false);
	m_table->clearContents();
	m_table->setRowCount(int(ordered.size()));
	for (int r = 0; r < ordered.size(); ++r)
	{
		const Item& i = ordered.at(r);
		auto* type = new QTableWidgetItem(i.icon, i.folder ? m_backend->text(QStringLiteral("folder.type.folder")) : i.type);
		type->setData(RoleItemId, i.id);
		type->setData(RoleItemFolder, i.folder);
		m_table->setItem(r, 0, type);
		m_table->setItem(r, 1, new QTableWidgetItem(i.name));
		if (i.folder)
			continue;
		m_table->setItem(r, 2, new QTableWidgetItem(i.host));
		const QJsonObject v = inv.value(i.id).toObject();
		for (int f = 0; f < int(std::size(kFields)); ++f)
		{
			auto* cell = new QTableWidgetItem(v.value(QString::fromLatin1(kFields[f])).toString());
			// Version und Seriennummer hervorgehoben (bei APs besonders wichtig).
			if (qstrcmp(kFields[f], "version") == 0 || qstrcmp(kFields[f], "serial") == 0)
				cell->setFont(strong);
			m_table->setItem(r, kFirstField + f, cell);
		}
		const qint64 at = v.value(QStringLiteral("at")).toInteger();
		m_table->setItem(r, kAtColumn, new QTableWidgetItem(at > 0 ? QDateTime::fromSecsSinceEpoch(at).toString(QStringLiteral("dd.MM.yyyy HH:mm")) : QString()));
	}
	m_table->resizeColumnsToContents();
	applyFilter();
}

void FolderOverviewTab::applyFilter()
{
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
