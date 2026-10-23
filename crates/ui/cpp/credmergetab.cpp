#include "credmergetab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QInputDialog>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTreeWidget>
#include <QtWidgets/QVBoxLayout>

namespace
{
constexpr int RoleEntries = Qt::UserRole + 1;
constexpr int RoleCreate = Qt::UserRole + 2;
constexpr int RoleSuggestion = Qt::UserRole + 3;
constexpr int RoleNode = Qt::UserRole + 4;
} // namespace

CredMergeTab::CredMergeTab(RjBackend* backend, const QStringList& scope, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_scope(scope)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setProperty("editor", true);
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);
	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(this, &hin);
	auto* caption = new QLabel(T("credmerge.head"), head);
	QFont tf = caption->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	caption->setFont(tf);
	hin->addWidget(caption);
	auto* intro = new QLabel(T("credmerge.intro"), head);
	intro->setObjectName(QStringLiteral("rjMuted"));
	intro->setWordWrap(true);
	hin->addWidget(intro);
	m_state = new QLabel(head);
	m_state->setWordWrap(true);
	hin->addWidget(m_state);
	auto* bh = new QHBoxLayout;
	m_merge = new QPushButton(T("credmerge.merge"), head);
	auto* again = new QPushButton(T("orphans.again"), head);
	auto* close = new QPushButton(T("mass.close"), head);
	m_merge->setEnabled(false);
	bh->addWidget(m_merge);
	bh->addWidget(again);
	bh->addStretch(1);
	bh->addWidget(close);
	hin->addLayout(bh);
	lay->addWidget(head);

	QVBoxLayout* lin = nullptr;
	auto* lc = rjui::card(this, &lin);
	m_list = new QTreeWidget(lc);
	m_list->setColumnCount(3);
	m_list->setHeaderLabels({ T("credmerge.col.what"), T("credmerge.col.where"), T("credmerge.col.target") });
	m_list->header()->setDefaultAlignment(Qt::AlignLeft | Qt::AlignVCenter);
	lin->addWidget(m_list, 1);
	lay->addWidget(lc, 1);

	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(again, &QPushButton::clicked, this, &CredMergeTab::load);
	connect(m_list, &QTreeWidget::itemSelectionChanged, this, [this] {
		QTreeWidgetItem* it = m_list->currentItem();
		while (it && it->parent())
			it = it->parent();
		m_merge->setEnabled(it != nullptr);
	});
	connect(m_list, &QTreeWidget::itemDoubleClicked, this, [this](QTreeWidgetItem* it) {
		if (it && !it->data(0, RoleNode).toString().isEmpty())
			emit showTarget(it->data(0, RoleNode).toString());
	});
	connect(m_merge, &QPushButton::clicked, this, [this] {
		QTreeWidgetItem* it = m_list->currentItem();
		while (it && it->parent())
			it = it->parent();
		if (it)
			apply(it->data(0, RoleEntries).toJsonArray(), it->data(0, RoleCreate).toBool(), it->data(0, RoleSuggestion).toString());
	});
	load();
}

QJsonObject CredMergeTab::call(const QJsonObject& cmd) const
{
	return QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
}

void CredMergeTab::load()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("credmerge_list") }, { QStringLiteral("ids"), QJsonArray::fromStringList(m_scope) } });
	const QJsonArray groups = r.value(QStringLiteral("groups")).toArray();
	m_list->clear();
	for (const QJsonValue& g : groups)
	{
		const QJsonObject o = g.toObject();
		const QJsonObject login = o.value(QStringLiteral("login")).toObject();
		auto* top = new QTreeWidgetItem(m_list);
		top->setText(0, T("credmerge.group").arg(o.value(QStringLiteral("username")).toString()).arg(o.value(QStringLiteral("count")).toInt()));
		top->setText(1, login.isEmpty() ? T("credmerge.new").arg(o.value(QStringLiteral("folder")).toString()) : T("credmerge.existing").arg(login.value(QStringLiteral("name")).toString()));
		top->setData(0, RoleEntries, o.value(QStringLiteral("entries")).toArray());
		top->setData(0, RoleCreate, login.isEmpty());
		top->setData(0, RoleSuggestion, o.value(QStringLiteral("suggestion")).toString());
		QFont bf = top->font(0);
		bf.setBold(true);
		top->setFont(0, bf);
		for (const QJsonValue& u : o.value(QStringLiteral("uses")).toArray())
		{
			const QJsonObject x = u.toObject();
			auto* child = new QTreeWidgetItem(top);
			child->setText(0, x.value(QStringLiteral("what")).toString());
			child->setText(1, x.value(QStringLiteral("path")).toString());
			child->setText(2, x.value(QStringLiteral("name")).toString());
			child->setData(0, RoleNode, x.value(QStringLiteral("id")).toString());
		}
	}
	m_list->expandAll();
	for (int c = 0; c < 3; ++c)
		m_list->resizeColumnToContents(c);
	m_merge->setEnabled(false);
	m_state->setText(r.value(QStringLiteral("locked")).toBool() ? T("orphans.locked") : groups.isEmpty() ? T("credmerge.none") : T("credmerge.found").arg(groups.size()));
}

void CredMergeTab::apply(const QJsonArray& entries, bool create, const QString& suggestion)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("credmerge_apply") }, { QStringLiteral("entries"), entries }, { QStringLiteral("ids"), QJsonArray::fromStringList(m_scope) } };
	if (create)
	{
		bool ok = false;
		const QString name = QInputDialog::getText(this, T("credmerge.name.title"), T("credmerge.name.ask"), QLineEdit::Normal, suggestion, &ok);
		if (!ok)
			return;
		cmd.insert(QStringLiteral("name"), name);
	}
	const QJsonObject r = call(cmd);
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(r.value(QStringLiteral("error")).toString());
		return;
	}
	emit indexChanged();
	load();
	m_state->setText(T("credmerge.done").arg(r.value(QStringLiteral("changed")).toInt()));
	emit status(m_state->text());
}
