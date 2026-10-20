#include "mergetab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
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

// Daten an den Zeilen: Gruppe → Hauptziel und Name; Ziel → ID.
constexpr int RoleId = Qt::UserRole;
constexpr int RoleKeep = Qt::UserRole + 1;
constexpr int RoleName = Qt::UserRole + 2;
const QString undoLabel = QStringLiteral("Ziele zusammenfassen");
} // namespace

MergeTab::MergeTab(RjBackend* backend, const QStringList& ids, const QString& title, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_ids(ids)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setObjectName(QStringLiteral("mergeTab"));
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	lay->setSpacing(10);

	// Kopfkarte: Auswahl, Stand, Knöpfe.
	QVBoxLayout* hin = nullptr;
	auto* head = rjui::card(this, &hin);
	auto* hh = new QHBoxLayout;
	auto* hv = new QVBoxLayout;
	hv->setSpacing(2);
	auto* caption = new QLabel(T("merge.head").arg(title), head);
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
	m_apply = new QPushButton(T("merge.apply"), head);
	m_apply->setDefault(true);
	m_undo = new QPushButton(T("merge.undo"), head);
	m_undo->setEnabled(false);
	auto* close = new QPushButton(T("mass.close"), head);
	hh->addWidget(m_apply, 0, Qt::AlignTop);
	hh->addWidget(m_undo, 0, Qt::AlignTop);
	hh->addWidget(close, 0, Qt::AlignTop);
	hin->addLayout(hh);
	lay->addWidget(head);

	// Karte mit den Gruppen; je Gruppe die Ziele darunter.
	QVBoxLayout* lin = nullptr;
	auto* listCard = rjui::card(this, &lin);
	lin->addWidget(rjui::section(T("merge.groups"), listCard));
	m_list = new QTreeWidget(listCard);
	m_list->setUniformRowHeights(true);
	m_list->setColumnCount(4);
	m_list->setHeaderLabels({ T("merge.col.target"), T("merge.col.access"), T("mass.col.path"), T("merge.col.role") });
	m_list->header()->setSectionResizeMode(0, QHeaderView::Interactive);
	m_list->header()->setSectionResizeMode(1, QHeaderView::ResizeToContents);
	m_list->header()->setSectionResizeMode(2, QHeaderView::Interactive);
	m_list->header()->setSectionResizeMode(3, QHeaderView::Stretch);
	m_list->setColumnWidth(0, 260);
	m_list->setColumnWidth(2, 220);
	lin->addWidget(m_list, 1);
	lay->addWidget(listCard, 1);

	// Karte der gewählten Gruppe: Hauptziel, Name, Hinweise.
	QVBoxLayout* gin = nullptr;
	auto* groupCard = rjui::card(this, &gin);
	m_groupHead = rjui::section(T("merge.group.none"), groupCard);
	gin->addWidget(m_groupHead);
	auto* form = new QFormLayout;
	m_keep = new QComboBox(groupCard);
	m_name = new QLineEdit(groupCard);
	m_name->setPlaceholderText(T("merge.name.keep"));
	form->addRow(T("merge.keep"), m_keep);
	form->addRow(T("merge.name"), m_name);
	gin->addLayout(form);
	m_notes = new QLabel(groupCard);
	m_notes->setWordWrap(true);
	m_notes->setTextFormat(Qt::PlainText);
	gin->addWidget(m_notes);
	auto* hint = new QLabel(T("merge.hint"), groupCard);
	hint->setObjectName(QStringLiteral("rjMuted"));
	hint->setWordWrap(true);
	gin->addWidget(hint);
	lay->addWidget(groupCard);

	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_apply, &QPushButton::clicked, this, &MergeTab::apply);
	connect(m_undo, &QPushButton::clicked, this, &MergeTab::undo);
	connect(m_list, &QTreeWidget::currentItemChanged, this, [this](QTreeWidgetItem* it) {
		select(it ? (it->parent() ? it->parent() : it) : nullptr);
	});
	connect(m_list, &QTreeWidget::itemDoubleClicked, this, [this](QTreeWidgetItem* it) {
		if (it->parent())
			emit showTarget(it->data(0, RoleId).toString());
	});
	connect(m_list, &QTreeWidget::itemChanged, this, [this](QTreeWidgetItem* it, int col) {
		if (m_filling || col != 0)
			return;
		QTreeWidgetItem* g = it->parent() ? it->parent() : it;
		const QString keep = g->data(0, RoleKeep).toString();
		m_filling = true;
		// Haken an der Gruppe: alle Ziele mit. Das Hauptziel bleibt immer dabei; die Gruppe gilt als gewählt,
		// solange ein weiteres Ziel angehakt ist.
		for (int i = 0; i < g->childCount(); ++i)
		{
			QTreeWidgetItem* c = g->child(i);
			if (!it->parent())
				c->setCheckState(0, it->checkState(0));
			if (c->data(0, RoleId).toString() == keep)
				c->setCheckState(0, Qt::Checked);
		}
		g->setCheckState(0, others(g).isEmpty() ? Qt::Unchecked : Qt::Checked);
		m_filling = false;
		updateRoles(g);
		if (g == m_current)
			check(g);
		updateState();
	});
	connect(m_keep, &QComboBox::currentIndexChanged, this, [this] {
		if (m_filling || !m_current)
			return;
		const QString keep = m_keep->currentData().toString();
		const QString oldKeep = m_current->data(0, RoleKeep).toString();
		// Bisheriger Name des Hauptziels: mit dem Hauptziel wechselt der vorgeschlagene Name.
		QString oldName, newName;
		for (int i = 0; i < m_current->childCount(); ++i)
		{
			QTreeWidgetItem* c = m_current->child(i);
			if (c->data(0, RoleId).toString() == oldKeep)
				oldName = c->data(0, RoleName).toString();
			if (c->data(0, RoleId).toString() == keep)
			{
				newName = c->data(0, RoleName).toString();
				m_filling = true;
				c->setCheckState(0, Qt::Checked);
				m_filling = false;
			}
		}
		m_current->setData(0, RoleKeep, keep);
		if (m_name->text() == oldName)
		{
			m_name->setText(newName);
			m_current->setData(0, RoleName, newName);
		}
		updateRoles(m_current);
		check(m_current);
		updateState();
	});
	connect(m_name, &QLineEdit::textEdited, this, [this](const QString& t) {
		if (m_current)
			m_current->setData(0, RoleName, t);
	});
	load();
}

QString MergeTab::edit(const QJsonObject& cmd) const
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

void MergeTab::load()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("merge_preview") }, { QStringLiteral("ids"), QJsonArray::fromStringList(m_ids) } }));
	m_filling = true;
	m_list->clear();
	m_current = nullptr;
	if (r.contains(QStringLiteral("error")))
		m_state->setText(r.value(QStringLiteral("error")).toString());
	for (const QJsonValue& gv : r.value(QStringLiteral("groups")).toArray())
	{
		const QJsonObject g = gv.toObject();
		const QJsonArray members = g.value(QStringLiteral("members")).toArray();
		auto* gi = new QTreeWidgetItem(m_list);
		gi->setText(0, T("merge.group").arg(g.value(QStringLiteral("address")).toString()).arg(members.size()));
		gi->setFlags(gi->flags() | Qt::ItemIsUserCheckable);
		gi->setCheckState(0, Qt::Checked);
		gi->setData(0, RoleKeep, g.value(QStringLiteral("keep")).toString());
		gi->setData(0, RoleName, g.value(QStringLiteral("name")).toString());
		QFont bf = gi->font(0);
		bf.setBold(true);
		gi->setFont(0, bf);
		for (const QJsonValue& mv : members)
		{
			const QJsonObject m = mv.toObject();
			auto* ci = new QTreeWidgetItem(gi);
			ci->setFlags(ci->flags() | Qt::ItemIsUserCheckable);
			ci->setCheckState(0, Qt::Checked);
			ci->setData(0, RoleId, m.value(QStringLiteral("id")).toString());
			ci->setData(0, RoleName, m.value(QStringLiteral("name")).toString());
			ci->setText(0, m.value(QStringLiteral("name")).toString());
			QString access = m.value(QStringLiteral("label")).toString() + QStringLiteral(" · ") + m.value(QStringLiteral("endpoint")).toString();
			if (const int w = m.value(QStringLiteral("ways")).toInt(); w > 0)
				access += T("merge.ways.n").arg(w);
			ci->setText(1, access);
			ci->setText(2, m.value(QStringLiteral("path")).toString());
			for (int c = 0; c < 3; ++c)
				ci->setToolTip(c, ci->text(c));
		}
		gi->setExpanded(true);
		updateRoles(gi);
	}
	m_filling = false;
	if (m_list->topLevelItemCount() > 0)
		m_list->setCurrentItem(m_list->topLevelItem(0));
	else
		select(nullptr);
	updateState();
}

QStringList MergeTab::others(QTreeWidgetItem* g) const
{
	QStringList out;
	if (!g)
		return out;
	const QString keep = g->data(0, RoleKeep).toString();
	for (int i = 0; i < g->childCount(); ++i)
	{
		QTreeWidgetItem* c = g->child(i);
		if (c->checkState(0) == Qt::Checked && c->data(0, RoleId).toString() != keep)
			out << c->data(0, RoleId).toString();
	}
	return out;
}

void MergeTab::updateRoles(QTreeWidgetItem* g)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const bool was = m_filling;
	m_filling = true;
	const QString keep = g->data(0, RoleKeep).toString();
	const bool active = !others(g).isEmpty();
	for (int i = 0; i < g->childCount(); ++i)
	{
		QTreeWidgetItem* c = g->child(i);
		const bool isKeep = c->data(0, RoleId).toString() == keep;
		QFont f = c->font(0);
		f.setBold(isKeep);
		c->setFont(0, f);
		c->setText(3, !active                               ? T("merge.role.stays")
		              : isKeep                              ? T("merge.role.keep")
		              : c->checkState(0) == Qt::Checked     ? T("merge.role.way")
		                                                    : T("merge.role.stays"));
	}
	g->setText(3, active ? QString() : T("merge.role.skip"));
	m_filling = was;
}

void MergeTab::select(QTreeWidgetItem* g)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	m_current = g;
	m_filling = true;
	m_keep->clear();
	m_keep->setEnabled(g != nullptr);
	m_name->setEnabled(g != nullptr);
	if (!g)
	{
		m_groupHead->setText(T("merge.group.none"));
		m_name->clear();
		m_notes->clear();
		m_filling = false;
		return;
	}
	m_groupHead->setText(g->text(0));
	for (int i = 0; i < g->childCount(); ++i)
	{
		QTreeWidgetItem* c = g->child(i);
		m_keep->addItem(c->text(0) + QStringLiteral(" (") + c->text(1).section(QStringLiteral(" · "), 0, 0) + QStringLiteral(")"), c->data(0, RoleId));
	}
	m_keep->setCurrentIndex(qMax(0, m_keep->findData(g->data(0, RoleKeep))));
	m_name->setText(g->data(0, RoleName).toString());
	m_filling = false;
	check(g);
}

void MergeTab::check(QTreeWidgetItem* g)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const QStringList o = others(g);
	if (o.isEmpty())
	{
		m_notes->setText(T("merge.notes.skip"));
		return;
	}
	const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("merge_check") }, { QStringLiteral("keep"), g->data(0, RoleKeep).toString() }, { QStringLiteral("others"), QJsonArray::fromStringList(o) } }));
	if (r.contains(QStringLiteral("error")))
	{
		m_notes->setText(r.value(QStringLiteral("error")).toString());
		return;
	}
	QStringList lines;
	lines << T("merge.notes.ways").arg(r.value(QStringLiteral("ways")).toInt());
	if (const int l = r.value(QStringLiteral("logins")).toInt(); l > 0)
		lines << T("merge.notes.logins").arg(l);
	for (const QJsonValue& n : r.value(QStringLiteral("notes")).toArray())
		lines << QStringLiteral("• ") + n.toString();
	m_notes->setText(lines.join(QLatin1Char('\n')));
}

void MergeTab::updateState()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	int groups = 0, chosen = 0, targets = 0;
	for (int i = 0; i < m_list->topLevelItemCount(); ++i)
	{
		QTreeWidgetItem* g = m_list->topLevelItem(i);
		++groups;
		if (const int n = others(g).size(); n > 0)
		{
			++chosen;
			targets += n;
		}
	}
	if (groups == 0)
		m_state->setText(T("merge.none"));
	else
		m_state->setText(T("merge.state").arg(groups).arg(chosen).arg(targets));
	m_apply->setEnabled(chosen > 0);
}

void MergeTab::apply()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	QJsonArray groups;
	int targets = 0;
	for (int i = 0; i < m_list->topLevelItemCount(); ++i)
	{
		QTreeWidgetItem* g = m_list->topLevelItem(i);
		const QStringList o = others(g);
		if (o.isEmpty())
			continue;
		targets += o.size();
		groups.append(QJsonObject { { QStringLiteral("keep"), g->data(0, RoleKeep).toString() },
		                            { QStringLiteral("others"), QJsonArray::fromStringList(o) },
		                            { QStringLiteral("name"), g->data(0, RoleName).toString() } });
	}
	if (groups.isEmpty())
		return;
	QMessageBox box(QMessageBox::Question, T("merge.apply.title"), T("merge.apply.ask").arg(groups.size()).arg(targets), QMessageBox::NoButton, this);
	auto* ok = box.addButton(T("merge.apply"), QMessageBox::AcceptRole);
	box.setDefaultButton(box.addButton(T("merge.apply.cancel"), QMessageBox::RejectRole));
	box.exec();
	if (box.clickedButton() != ok)
		return;
	const QJsonObject r = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("merge_apply") }, { QStringLiteral("groups"), groups } }));
	if (r.contains(QStringLiteral("error")))
	{
		QMessageBox::warning(this, T("merge.apply.title"), r.value(QStringLiteral("error")).toString());
		return;
	}
	emit indexChanged();
	load();
	QString done = T("merge.done").arg(groups.size()).arg(r.value(QStringLiteral("merged")).toInt()).arg(r.value(QStringLiteral("ways")).toInt());
	if (const int l = r.value(QStringLiteral("logins")).toInt(); l > 0)
		done += T("merge.done.logins").arg(l);
	m_done = done;
	m_state->setText(done);
	emit status(done);
	m_undo->setEnabled(true);
}

void MergeTab::undo()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	// Nur den eigenen Schritt zurücknehmen; dazwischen geänderte Dinge nicht anfassen.
	const QJsonObject l = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("undo_label") } }));
	if (l.value(QStringLiteral("label")).toString() != undoLabel)
	{
		QMessageBox::information(this, T("merge.undo"), T("merge.undo.other"));
		m_undo->setEnabled(false);
		return;
	}
	edit({ { QStringLiteral("op"), QStringLiteral("undo") } });
	emit indexChanged();
	m_undo->setEnabled(false);
	load();
	m_state->setText(T("merge.undone"));
}

void MergeTab::refresh()
{
	// Der eigene Schritt ist nur noch zurücknehmbar, solange er der letzte im Verlauf ist.
	const QJsonObject l = parseObj(edit({ { QStringLiteral("op"), QStringLiteral("undo_label") } }));
	const bool mine = m_undo->isEnabled() && l.value(QStringLiteral("label")).toString() == undoLabel;
	load();
	m_undo->setEnabled(mine);
	if (mine)
		m_state->setText(m_done);
}
