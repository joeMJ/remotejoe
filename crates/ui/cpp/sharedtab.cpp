#include "sharedtab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTreeWidget>
#include <QtWidgets/QVBoxLayout>

namespace
{
QJsonObject parseObj(const QString& s)
{
	return QJsonDocument::fromJson(s.toUtf8()).object();
}
constexpr int RoleEntry = Qt::UserRole;
constexpr int RoleId = Qt::UserRole + 1;
} // namespace

SharedCredTab::SharedCredTab(RjBackend* backend, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setProperty("editor", true);
	setObjectName(QStringLiteral("sharedCredTab"));
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);

	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(this, &hin);
	auto* hh = new QHBoxLayout;
	auto* hv = new QVBoxLayout;
	hv->setSpacing(2);
	auto* caption = new QLabel(T("shared.head"), head);
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
	auto* again = new QPushButton(T("shared.again"), head);
	auto* close = new QPushButton(T("mass.close"), head);
	hh->addWidget(again, 0, Qt::AlignTop);
	hh->addWidget(close, 0, Qt::AlignTop);
	hin->addLayout(hh);
	lay->addWidget(head);

	QVBoxLayout* lin = nullptr;
	auto* card = rjui::card(this, &lin);
	lin->addWidget(rjui::section(T("shared.groups"), card));
	m_list = new QTreeWidget(card);
	m_list->setColumnCount(3);
	m_list->setHeaderLabels({ T("shared.col.target"), T("mass.col.path"), T("shared.col.what") });
	m_list->header()->setSectionResizeMode(0, QHeaderView::Interactive);
	m_list->header()->setSectionResizeMode(1, QHeaderView::Interactive);
	m_list->header()->setSectionResizeMode(2, QHeaderView::Stretch);
	m_list->setColumnWidth(0, 300);
	m_list->setColumnWidth(1, 260);
	lin->addWidget(m_list, 1);
	auto* bh = new QHBoxLayout;
	m_split = new QPushButton(T("shared.split"), card);
	m_login = new QPushButton(T("shared.login"), card);
	bh->addWidget(m_split);
	bh->addWidget(m_login);
	bh->addStretch(1);
	lin->addLayout(bh);
	auto* hint = new QLabel(T("shared.hint"), card);
	hint->setObjectName(QStringLiteral("rjMuted"));
	hint->setWordWrap(true);
	lin->addWidget(hint);
	lay->addWidget(card, 1);

	connect(again, &QPushButton::clicked, this, &SharedCredTab::load);
	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_split, &QPushButton::clicked, this, [this] { act("shared_split"); });
	connect(m_login, &QPushButton::clicked, this, [this] { act("shared_to_login"); });
	connect(m_list, &QTreeWidget::itemDoubleClicked, this, [this](QTreeWidgetItem* it) {
		if (it->parent())
			emit showTarget(it->data(0, RoleId).toString());
	});
	connect(m_list, &QTreeWidget::currentItemChanged, this, [this](QTreeWidgetItem* it) {
		m_split->setEnabled(it != nullptr);
		m_login->setEnabled(it != nullptr);
	});
	load();
}

QString SharedCredTab::edit(const QJsonObject& cmd) const
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

void SharedCredTab::load()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("shared_list") } }));
	m_list->clear();
	const QJsonArray groups = r.value(QStringLiteral("groups")).toArray();
	for (const QJsonValue& gv : groups)
	{
		const QJsonObject g = gv.toObject();
		const QJsonArray users = g.value(QStringLiteral("users")).toArray();
		auto* gi = new QTreeWidgetItem(m_list);
		gi->setText(0, T("shared.group").arg(g.value(QStringLiteral("title")).toString(), g.value(QStringLiteral("username")).toString()).arg(users.size()));
		gi->setData(0, RoleEntry, g.value(QStringLiteral("entry")).toString());
		QFont bf = gi->font(0);
		bf.setBold(true);
		gi->setFont(0, bf);
		for (const QJsonValue& uv : users)
		{
			const QJsonObject u = uv.toObject();
			auto* ci = new QTreeWidgetItem(gi);
			ci->setText(0, u.value(QStringLiteral("name")).toString());
			ci->setText(1, u.value(QStringLiteral("path")).toString());
			ci->setText(2, u.value(QStringLiteral("what")).toString());
			ci->setData(0, RoleId, u.value(QStringLiteral("id")).toString());
			for (int c = 0; c < 3; ++c)
				ci->setToolTip(c, ci->text(c));
		}
		gi->setExpanded(true);
	}
	m_state->setText(r.value(QStringLiteral("locked")).toBool() ? T("shared.locked")
	                 : groups.isEmpty()                         ? T("shared.none")
	                                                            : T("shared.found").arg(groups.size()));
	if (m_list->topLevelItemCount() > 0)
		m_list->setCurrentItem(m_list->topLevelItem(0));
	m_split->setEnabled(m_list->currentItem() != nullptr);
	m_login->setEnabled(m_list->currentItem() != nullptr);
}

void SharedCredTab::act(const char* op)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	QTreeWidgetItem* it = m_list->currentItem();
	if (!it)
		return;
	QTreeWidgetItem* g = it->parent() ? it->parent() : it;
	const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QString::fromLatin1(op) }, { QStringLiteral("entry"), g->data(0, RoleEntry).toString() } }));
	if (r.contains(QStringLiteral("error")))
	{
		QMessageBox::warning(this, T("shared.head"), r.value(QStringLiteral("error")).toString());
		return;
	}
	emit indexChanged();
	emit status(T(qstrcmp(op, "shared_split") == 0 ? "shared.done.split" : "shared.done.login"));
	load();
}
