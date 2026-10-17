#include "editor.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtGui/QKeyEvent>
#include <QtGui/QPainter>
#include <QtGui/QPainterPath>
#include <QtGui/QShortcut>
#include <QtGui/QFileSystemModel>
#include <QtWidgets/QButtonGroup>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QCompleter>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QListWidget>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QRadioButton>
#include <QtWidgets/QScrollArea>
#include <QtWidgets/QSplitter>
#include <QtWidgets/QStackedWidget>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QTreeWidget>
#include <QtWidgets/QVBoxLayout>

QString rjText(RjBackend* b, const char* key)
{
	return b->text(QString::fromLatin1(key));
}

namespace {

QJsonObject parse(const QString& s)
{
	return QJsonDocument::fromJson(s.toUtf8()).object();
}

QString S(const char* s)
{
	return QString::fromUtf8(s);
}

/// Kleines Verlaufsdiagramm (SNMP).
class Sparkline : public QWidget
{
public:
	Sparkline(const QString& title, const QJsonArray& values, const QString& unit, QWidget* parent)
	  : QWidget(parent)
	  , m_title(title)
	  , m_unit(unit)
	{
		for (const QJsonValue& v : values)
			m_values << v.toDouble();
		setMinimumSize(260, 56);
	}

protected:
	void paintEvent(QPaintEvent*) override
	{
		QPainter p(this);
		p.setRenderHint(QPainter::Antialiasing);
		p.fillRect(rect(), palette().base());
		p.setPen(palette().text().color());
		const QString last = m_values.isEmpty() ? QStringLiteral("–") : QString::number(m_values.last(), 'f', 1) + m_unit;
		p.drawText(rect().adjusted(4, 2, -4, -2), Qt::AlignTop | Qt::AlignLeft, m_title + QStringLiteral(": ") + last);
		if (m_values.size() < 2)
			return;
		double mx = 1;
		for (double v : m_values)
			mx = qMax(mx, v);
		const QRectF area = QRectF(rect()).adjusted(4, 20, -4, -4);
		QPainterPath path;
		for (int i = 0; i < m_values.size(); ++i)
		{
			const QPointF pt(area.left() + area.width() * i / (m_values.size() - 1), area.bottom() - area.height() * m_values[i] / mx);
			i == 0 ? path.moveTo(pt) : path.lineTo(pt);
		}
		p.setPen(QPen(palette().highlight().color(), 2));
		p.drawPath(path);
	}

private:
	QString m_title, m_unit;
	QVector<double> m_values;
};

QLabel* errorLabel(QWidget* parent)
{
	auto* l = new QLabel(parent);
	l->setObjectName(QStringLiteral("fieldError"));
	l->setWordWrap(true);
	l->hide();
	return l;
}

void setWarn(QLabel* l, const QString& text)
{
	l->setText(text);
	l->setVisible(!text.isEmpty());
}

const char* const kClip[] = { "off", "host_to_target", "target_to_host", "both" };

} // namespace

// ---------------------------------------------------------------- ActionBar

ActionBar::ActionBar(RjBackend* backend, QWidget* parent)
  : QFrame(parent)
  , m_backend(backend)
{
	setObjectName(QStringLiteral("actionBar"));
	setFrameShape(QFrame::StyledPanel);
	auto* lay = new QHBoxLayout(this);
	lay->setContentsMargins(6, 4, 6, 4);
	m_label = new QLabel(this);
	m_label->setWordWrap(true);
	m_line = new QLineEdit(this);
	m_combo = new QComboBox(this);
	m_check = new QCheckBox(this);
	m_label->setBuddy(m_line);
	lay->addWidget(m_label, 1);
	lay->addWidget(m_line, 1);
	lay->addWidget(m_combo, 1);
	lay->addWidget(m_check);
	for (int i = 0; i < 3; ++i)
	{
		auto* b = new QPushButton(this);
		connect(b, &QPushButton::clicked, this, [this, i] {
			auto done = m_done;
			hide();
			if (done)
				done(i);
		});
		lay->addWidget(b);
		m_buttons << b;
	}
	m_cancel = new QPushButton(rjText(backend, "bar.cancel"), this);
	connect(m_cancel, &QPushButton::clicked, this, [this] {
		auto done = m_done;
		hide();
		if (done)
			done(-1);
	});
	lay->addWidget(m_cancel);
	connect(m_line, &QLineEdit::returnPressed, m_buttons[0], &QPushButton::click);
	hide();
}

void ActionBar::reset(const QString& text)
{
	m_label->setText(text);
	m_line->hide();
	m_combo->hide();
	m_check->hide();
	for (auto* b : m_buttons)
		b->hide();
	m_done = nullptr;
	show();
}

void ActionBar::ask(const QString& text, const QStringList& buttons, std::function<void(int)> done)
{
	reset(text);
	for (int i = 0; i < buttons.size() && i < m_buttons.size(); ++i)
	{
		m_buttons[i]->setText(buttons[i]);
		m_buttons[i]->show();
	}
	m_done = std::move(done);
	(buttons.isEmpty() ? m_cancel : m_buttons[0])->setFocus();
}

void ActionBar::prompt(const QString& text, const QString& value, std::function<void(const QString&)> done)
{
	reset(text);
	m_line->setText(value);
	m_line->show();
	m_line->selectAll();
	m_buttons[0]->setText(rjText(m_backend, "bar.ok"));
	m_buttons[0]->show();
	m_done = [this, done](int i) {
		if (i == 0)
			done(m_line->text());
	};
	m_line->setFocus();
}

void ActionBar::choose(const QString& text, const QList<QPair<QString, QString>>& items, std::function<void(const QString&)> done,
                       const QString& checkText, std::function<void(const QString&, bool)> doneCheck)
{
	reset(text);
	m_label->setBuddy(m_combo);
	m_combo->clear();
	for (const auto& it : items)
		m_combo->addItem(it.first, it.second);
	m_combo->show();
	if (!checkText.isEmpty())
	{
		m_check->setText(checkText);
		m_check->setChecked(false);
		m_check->show();
	}
	m_buttons[0]->setText(rjText(m_backend, "bar.ok"));
	m_buttons[0]->show();
	m_done = [this, done, doneCheck](int i) {
		if (i != 0)
			return;
		if (doneCheck)
			doneCheck(m_combo->currentData().toString(), m_check->isChecked());
		else
			done(m_combo->currentData().toString());
	};
	m_combo->setFocus();
}

void ActionBar::dismiss()
{
	m_done = nullptr;
	hide();
}

void ActionBar::keyPressEvent(QKeyEvent* e)
{
	if (e->key() == Qt::Key_Escape)
	{
		m_cancel->click();
		return;
	}
	QFrame::keyPressEvent(e);
}

// ------------------------------------------------------------ PropertiesTab

PropertiesTab::PropertiesTab(RjBackend* backend, const QString& id, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
  , m_id(id)
{
	setProperty("editor", true);
	setObjectName(QStringLiteral("propertiesTab"));
	auto* sc = new QShortcut(QKeySequence(QStringLiteral("Ctrl+S")), this);
	sc->setContext(Qt::WidgetWithChildrenShortcut);
	connect(sc, &QShortcut::activated, this, &PropertiesTab::save);
	reload();
}

QString PropertiesTab::edit(const QJsonObject& cmd)
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

QString PropertiesTab::title() const
{
	return m_data.value(QStringLiteral("name_check")).toObject().value(QStringLiteral("marked")).toString() + (isDirty() ? QStringLiteral(" •") : QString());
}

void PropertiesTab::reload()
{
	m_data = parse(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), m_id } }));
	const QString current = m_cats && m_cats->currentItem() ? m_cats->currentItem()->data(0, Qt::UserRole).toString() : QString();
	build();
	if (!current.isEmpty())
		selectCategory(current);
	emit titleChanged();
}

void PropertiesTab::addPage(const QString& key, const char* label, QWidget* page)
{
	auto* scroll = new QScrollArea(m_pages);
	scroll->setWidgetResizable(true);
	scroll->setFrameShape(QFrame::NoFrame);
	scroll->setWidget(page);
	m_pages->addWidget(scroll);
	auto* item = new QTreeWidgetItem(m_cats, QStringList(rjText(m_backend, label)));
	item->setData(0, Qt::UserRole, key);
}

void PropertiesTab::selectCategory(const QString& key)
{
	for (int i = 0; i < m_cats->topLevelItemCount(); ++i)
		if (m_cats->topLevelItem(i)->data(0, Qt::UserRole).toString() == key)
			m_cats->setCurrentItem(m_cats->topLevelItem(i));
}

void PropertiesTab::track(QWidget* w, const QString& group)
{
	w->setProperty("rjGroup", group);
	if (auto* l = qobject_cast<QLineEdit*>(w))
		connect(l, &QLineEdit::textEdited, this, [this, group] { markDirty(group); });
	else if (auto* c = qobject_cast<QComboBox*>(w))
	{
		connect(c, &QComboBox::currentIndexChanged, this, [this, group] { markDirty(group); });
		if (c->isEditable())
			connect(c, &QComboBox::editTextChanged, this, [this, group] { markDirty(group); });
	}
	else if (auto* b = qobject_cast<QAbstractButton*>(w))
		connect(b, &QAbstractButton::toggled, this, [this, group] { markDirty(group); });
	else if (auto* p = qobject_cast<QPlainTextEdit*>(w))
		connect(p, &QPlainTextEdit::textChanged, this, [this, group] { markDirty(group); });
	else if (auto* t = qobject_cast<QTableWidget*>(w))
		connect(t, &QTableWidget::itemChanged, this, [this, group] { markDirty(group); });
}

void PropertiesTab::markDirty(const QString& group)
{
	if (m_loading)
		return;
	const bool was = isDirty();
	m_dirty.insert(group);
	// Geänderte Felder sichtbar markieren.
	for (QWidget* w : findChildren<QWidget*>())
		if (w->property("rjGroup").toString() == group)
		{
			w->setProperty("rjDirty", true);
			w->style()->unpolish(w);
			w->style()->polish(w);
		}
	m_dirtyLabel->show();
	if (!was)
		emit titleChanged();
}

void PropertiesTab::clearDirty()
{
	m_dirty.clear();
	emit titleChanged();
}

void PropertiesTab::build()
{
	m_loading = true;
	m_dirty.clear();
	m_errors.clear();
	m_opts.clear();
	delete layout();
	qDeleteAll(findChildren<QWidget*>(QString(), Qt::FindDirectChildrenOnly));
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(6, 6, 6, 6);
	m_bar = new ActionBar(m_backend, this);
	lay->addWidget(m_bar);
	if (m_data.contains(QStringLiteral("error")))
	{
		lay->addWidget(new QLabel(rjText(m_backend, "edit.error").arg(m_data.value(QStringLiteral("error")).toString()), this));
		m_loading = false;
		return;
	}
	auto* split = new QSplitter(Qt::Horizontal, this);
	m_cats = new QTreeWidget(split);
	m_cats->setHeaderHidden(true);
	m_cats->setRootIsDecorated(false);
	m_cats->setAccessibleName(QStringLiteral("Kategorien"));
	m_cats->setMaximumWidth(260);
	m_pages = new QStackedWidget(split);
	split->setStretchFactor(1, 1);
	lay->addWidget(split, 1);
	const QString kind = m_data.value(QStringLiteral("kind")).toString();
	const bool target = kind == QLatin1String("target");
	addPage(QStringLiteral("general"), "edit.cat.general", pageGeneral());
	addPage(QStringLiteral("connection"), "edit.cat.connection", pageConnection());
	addPage(QStringLiteral("credentials"), "edit.cat.credentials", pageCredentials());
	addPage(QStringLiteral("tunnel"), "edit.cat.tunnel", pageTunnel());
	addPage(QStringLiteral("tasks"), "edit.cat.tasks", pageTasks());
	addPage(QStringLiteral("clipboard"), "edit.cat.clipboard", pageClipboard());
	addPage(QStringLiteral("organisation"), "edit.cat.organisation", pageOrganisation());
	if (kind != QLatin1String("root"))
		addPage(QStringLiteral("parent"), "edit.cat.parent", pageParent());
	connect(m_cats, &QTreeWidget::currentItemChanged, this, [this](QTreeWidgetItem* cur) {
		if (cur)
			m_pages->setCurrentIndex(m_cats->indexOfTopLevelItem(cur));
	});
	m_cats->setCurrentItem(m_cats->topLevelItem(0));

	auto* bottom = new QHBoxLayout;
	m_dirtyLabel = new QLabel(rjText(m_backend, "edit.dirty"), this);
	m_dirtyLabel->setObjectName(QStringLiteral("dirtyLabel"));
	m_dirtyLabel->hide();
	m_status = new QLabel(this);
	m_status->setWordWrap(true);
	auto* saveB = new QPushButton(rjText(m_backend, "edit.save"), this);
	auto* discardB = new QPushButton(rjText(m_backend, "edit.discard"), this);
	saveB->setDefault(true);
	connect(saveB, &QPushButton::clicked, this, &PropertiesTab::save);
	connect(discardB, &QPushButton::clicked, this, [this] {
		reload();
		emit status(rjText(m_backend, "bar.discard"));
	});
	bottom->addWidget(m_dirtyLabel);
	bottom->addWidget(m_status, 1);
	if (target)
	{
		auto* tplB = new QPushButton(rjText(m_backend, "edit.template"), this);
		connect(tplB, &QPushButton::clicked, this, &PropertiesTab::saveTemplate);
		bottom->addWidget(tplB);
	}
	bottom->addWidget(discardB);
	bottom->addWidget(saveB);
	lay->addLayout(bottom);
	updateCredentialMode();
	m_loading = false;
}

QWidget* PropertiesTab::pageGeneral()
{
	auto* w = new QWidget;
	auto* f = new QFormLayout(w);
	m_name = new QLineEdit(m_data.value(QStringLiteral("name")).toString(), w);
	m_name->setAccessibleName(rjText(m_backend, "edit.name").remove(QLatin1Char('&')));
	m_nameWarn = errorLabel(w);
	const QJsonObject nc = m_data.value(QStringLiteral("name_check")).toObject();
	if (nc.value(QStringLiteral("issues")).toInt() > 0)
		setWarn(m_nameWarn, rjText(m_backend, "edit.charset.warn").arg(nc.value(QStringLiteral("marked")).toString()));
	connect(m_name, &QLineEdit::textEdited, this, [this](const QString& t) {
		const QJsonObject c = parse(edit({ { QStringLiteral("op"), QStringLiteral("check_text") }, { QStringLiteral("text"), t } }));
		setWarn(m_nameWarn, c.value(QStringLiteral("issues")).toInt() > 0 ? rjText(m_backend, "edit.charset.warn").arg(c.value(QStringLiteral("marked")).toString()) : QString());
	});
	track(m_name, QStringLiteral("name"));
	auto* nameLabel = new QLabel(rjText(m_backend, "edit.name"), w);
	nameLabel->setBuddy(m_name);
	f->addRow(nameLabel, m_name);
	f->addRow(QString(), m_nameWarn);
	m_errors.insert(QStringLiteral("name"), errorLabel(w));
	f->addRow(QString(), m_errors.value(QStringLiteral("name")));
	m_desc = new QLineEdit(m_data.value(QStringLiteral("description")).toString(), w);
	track(m_desc, QStringLiteral("description"));
	auto* dl = new QLabel(rjText(m_backend, "edit.description"), w);
	dl->setBuddy(m_desc);
	f->addRow(dl, m_desc);
	m_fav = new QCheckBox(rjText(m_backend, "edit.favorite"), w);
	m_fav->setChecked(m_data.value(QStringLiteral("favorite")).toBool());
	if (m_data.value(QStringLiteral("kind")).toString() == QLatin1String("root"))
		m_fav->setEnabled(false);
	track(m_fav, QStringLiteral("favorite"));
	f->addRow(QString(), m_fav);
	if (m_data.value(QStringLiteral("kind")).toString() == QLatin1String("target"))
	{
		f->addRow(rjText(m_backend, "edit.protocol") + QLatin1Char(':'), new QLabel(m_data.value(QStringLiteral("protocol")).toString().toUpper(), w));
		const QJsonObject p = parse(m_backend->propertiesJson(m_id));
		f->addRow(rjText(m_backend, "edit.path") + QLatin1Char(':'), new QLabel(p.value(QStringLiteral("path")).toString(), w));
		if (p.contains(QStringLiteral("snmp")))
		{
			const QJsonObject s = p.value(QStringLiteral("snmp")).toObject();
			const QString light = s.value(QStringLiteral("light")).toString();
			const QString state = light == QLatin1String("green") ? S("grün") : light == QLatin1String("yellow") ? S("gelb") : light == QLatin1String("red") ? S("rot") : S("grau (keine Antwort)");
			f->addRow(QStringLiteral("SNMP:"), new QLabel(state, w));
			f->addRow(new Sparkline(QStringLiteral("CPU"), s.value(QStringLiteral("cpu")).toArray(), QStringLiteral(" %"), w));
			f->addRow(new Sparkline(QStringLiteral("Speicher"), s.value(QStringLiteral("mem")).toArray(), QStringLiteral(" %"), w));
			f->addRow(new Sparkline(QStringLiteral("Durchsatz ein"), s.value(QStringLiteral("in")).toArray(), QStringLiteral(" bit/s"), w));
			f->addRow(new Sparkline(QStringLiteral("Durchsatz aus"), s.value(QStringLiteral("out")).toArray(), QStringLiteral(" bit/s"), w));
		}
	}
	return w;
}

QWidget* PropertiesTab::pageConnection()
{
	auto* w = new QWidget;
	auto* f = new QFormLayout(w);
	const bool target = m_data.value(QStringLiteral("kind")).toString() == QLatin1String("target");
	if (target)
	{
		const QString proto = m_data.value(QStringLiteral("protocol")).toString();
		m_host = new QLineEdit(m_data.value(QStringLiteral("host")).toString(), w);
		track(m_host, QStringLiteral("host"));
		m_hostWarn = errorLabel(w);
		connect(m_host, &QLineEdit::textEdited, this, [this](const QString& t) {
			const QJsonObject c = parse(edit({ { QStringLiteral("op"), QStringLiteral("check_text") }, { QStringLiteral("text"), t } }));
			setWarn(m_hostWarn, c.value(QStringLiteral("issues")).toInt() > 0 ? rjText(m_backend, "edit.charset.warn").arg(c.value(QStringLiteral("marked")).toString()) : QString());
		});
		auto* hl = new QLabel(rjText(m_backend, "edit.host"), w);
		hl->setBuddy(m_host);
		f->addRow(hl, m_host);
		f->addRow(QString(), m_hostWarn);
		m_errors.insert(QStringLiteral("host"), errorLabel(w));
		f->addRow(QString(), m_errors.value(QStringLiteral("host")));
		const QJsonValue port = m_data.value(QStringLiteral("port"));
		m_port = new QLineEdit(port.isDouble() ? QString::number(port.toInt()) : QString(), w);
		m_port->setPlaceholderText(rjText(m_backend, "edit.port.default").arg(m_data.value(QStringLiteral("default_port")).toInt()));
		track(m_port, QStringLiteral("port"));
		auto* pl = new QLabel(rjText(m_backend, "edit.port"), w);
		pl->setBuddy(m_port);
		f->addRow(pl, m_port);
		m_errors.insert(QStringLiteral("port"), errorLabel(w));
		f->addRow(QString(), m_errors.value(QStringLiteral("port")));
		// Protokollspezifische Felder (Zwischenablage auf eigener Seite).
		const QJsonObject o = m_data.value(QStringLiteral("options")).toObject();
		auto text = [&](const char* key, const char* label) {
			const QString k = QString::fromLatin1(key);
			const QJsonValue v = o.value(k);
			auto* e = new QLineEdit(v.isDouble() ? QString::number(v.toInteger()) : v.toString(), w);
			e->setProperty("numeric", v.isDouble() || k == QLatin1String("width") || k == QLatin1String("height") || k == QLatin1String("keepalive_secs"));
			track(e, QStringLiteral("options"));
			f->addRow(rjText(m_backend, label), e);
			m_opts.insert(k, e);
			m_errors.insert(k, errorLabel(w));
			f->addRow(QString(), m_errors.value(k));
		};
		auto check = [&](const char* key, const char* label) {
			auto* c = new QCheckBox(rjText(m_backend, label), w);
			c->setChecked(o.value(QString::fromLatin1(key)).toBool());
			track(c, QStringLiteral("options"));
			f->addRow(QString(), c);
			m_opts.insert(QString::fromLatin1(key), c);
		};
		if (proto == QLatin1String("rdp") || proto == QLatin1String("hyperv"))
		{
			text("width", "edit.opt.width");
			text("height", "edit.opt.height");
			text("keyboard_layout", "edit.opt.keyboard_layout");
			check("ignore_certificate", "edit.opt.ignore_certificate");
		}
		else if (proto == QLatin1String("ssh"))
		{
			text("term", "edit.opt.term");
			text("keepalive_secs", "edit.opt.keepalive_secs");
		}
		else if (proto == QLatin1String("vnc"))
			check("view_only", "edit.opt.view_only");
		else if (proto == QLatin1String("web") || proto == QLatin1String("proxmox"))
		{
			text("url", "edit.opt.url");
			text("user_selector", "edit.opt.user_selector");
			text("password_selector", "edit.opt.password_selector");
			text("submit_selector", "edit.opt.submit_selector");
			check("ignore_certificate", "edit.opt.ignore_certificate");
		}
	}
	// PowerShell-Transport und SNMP (vererbbar).
	m_psrp = new QComboBox(w);
	m_psrp->addItem(rjText(m_backend, "edit.psrp.inherit"), QStringLiteral("inherit"));
	m_psrp->addItem(rjText(m_backend, "edit.psrp.winrm_https"), QStringLiteral("winrm_https"));
	m_psrp->addItem(rjText(m_backend, "edit.psrp.winrm_http"), QStringLiteral("winrm_http"));
	m_psrp->addItem(rjText(m_backend, "edit.psrp.ssh"), QStringLiteral("ssh"));
	m_psrpAuth = new QComboBox(w);
	for (const char* a : { "kerberos", "ntlm", "basic" })
		m_psrpAuth->addItem(QString::fromLatin1(a).toUpper(), QString::fromLatin1(a));
	m_psrpPort = new QLineEdit(w);
	m_psrpVerify = new QCheckBox(rjText(m_backend, "edit.psrp.verify"), w);
	m_psrpVerify->setChecked(true);
	const QJsonValue pt = m_data.value(QStringLiteral("psrp_transport"));
	if (pt.isObject())
	{
		const QJsonObject t = pt.toObject();
		if (t.value(QStringLiteral("type")).toString() == QLatin1String("ssh"))
			m_psrp->setCurrentIndex(3);
		else
		{
			m_psrp->setCurrentIndex(t.value(QStringLiteral("https")).toBool(true) ? 1 : 2);
			m_psrpAuth->setCurrentIndex(qMax(0, m_psrpAuth->findData(t.value(QStringLiteral("auth")).toString())));
			m_psrpVerify->setChecked(t.value(QStringLiteral("verify_certificate")).toBool(true));
		}
		if (t.value(QStringLiteral("port")).isDouble())
			m_psrpPort->setText(QString::number(t.value(QStringLiteral("port")).toInt()));
	}
	for (QWidget* x : { static_cast<QWidget*>(m_psrp), static_cast<QWidget*>(m_psrpAuth), static_cast<QWidget*>(m_psrpPort), static_cast<QWidget*>(m_psrpVerify) })
		track(x, QStringLiteral("psrp"));
	auto* psl = new QLabel(rjText(m_backend, "edit.psrp"), w);
	psl->setBuddy(m_psrp);
	f->addRow(psl, m_psrp);
	f->addRow(rjText(m_backend, "edit.psrp.auth"), m_psrpAuth);
	f->addRow(rjText(m_backend, "edit.port"), m_psrpPort);
	f->addRow(QString(), m_psrpVerify);
	m_snmp = new QComboBox(w);
	m_snmp->addItem(rjText(m_backend, "edit.inherit"), QStringLiteral("inherit"));
	m_snmp->addItem(rjText(m_backend, "edit.none"), QStringLiteral("none"));
	for (const QJsonValue& d : m_data.value(QStringLiteral("devices")).toArray())
		m_snmp->addItem(d.toObject().value(QStringLiteral("name")).toString(), d.toObject().value(QStringLiteral("id")).toString());
	m_snmp->setCurrentIndex(qMax(0, m_snmp->findData(m_data.value(QStringLiteral("snmp")).toString())));
	track(m_snmp, QStringLiteral("snmp"));
	f->addRow(rjText(m_backend, "edit.snmp"), m_snmp);
	if (!target)
	{
		auto* a1 = new QPushButton(rjText(m_backend, "edit.apply") + QStringLiteral(" (") + rjText(m_backend, "edit.psrp") + QLatin1Char(')'), w);
		connect(a1, &QPushButton::clicked, this, [this] { applyToChildren(QStringLiteral("psrp_transport"), rjText(m_backend, "edit.psrp")); });
		auto* a2 = new QPushButton(rjText(m_backend, "edit.apply") + QStringLiteral(" (SNMP)"), w);
		connect(a2, &QPushButton::clicked, this, [this] { applyToChildren(QStringLiteral("snmp"), QStringLiteral("SNMP")); });
		f->addRow(QString(), a1);
		f->addRow(QString(), a2);
	}
	return w;
}

QWidget* PropertiesTab::pageCredentials()
{
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	const QJsonObject c = m_data.value(QStringLiteral("credential")).toObject();
	const bool unlocked = m_data.value(QStringLiteral("unlocked")).toBool();
	m_credMode = new QButtonGroup(w);
	const char* modes[][2] = { { "inherit", "edit.cred.inherit" }, { "none", "edit.cred.none" }, { "direct", "edit.cred.direct" },
		                       { "entry", "edit.cred.entry" },     { "named", "edit.cred.named" } };
	int id = 0;
	for (auto& m : modes)
	{
		auto* r = new QRadioButton(rjText(m_backend, m[1]), w);
		r->setProperty("mode", QString::fromLatin1(m[0]));
		r->setObjectName(QStringLiteral("cred_") + QString::fromLatin1(m[0]));
		m_credMode->addButton(r, id++);
		v->addWidget(r);
		track(r, QStringLiteral("credential"));
		const QString mode = QString::fromLatin1(m[0]);
		if (mode == QLatin1String("direct"))
		{
			m_directBox = new QWidget(w);
			auto* f = new QFormLayout(m_directBox);
			f->setContentsMargins(24, 0, 0, 0);
			m_user = new QLineEdit(c.value(QStringLiteral("username")).toString(), m_directBox);
			m_password = new QLineEdit(m_directBox);
			m_password->setEchoMode(QLineEdit::Password);
			m_password->setPlaceholderText(c.value(QStringLiteral("mode")).toString() == QLatin1String("direct") ? rjText(m_backend, "edit.cred.password.keep") : QString());
			m_password->setObjectName(QStringLiteral("passwordField"));
			m_pwWarn = errorLabel(m_directBox);
			// Kennwort wird nur geprüft und gemeldet, nie verändert.
			connect(m_password, &QLineEdit::textEdited, this, [this](const QString& t) {
				const QJsonObject r = parse(edit({ { QStringLiteral("op"), QStringLiteral("check_text") }, { QStringLiteral("text"), t } }));
				setWarn(m_pwWarn, r.value(QStringLiteral("issues")).toInt() > 0 ? rjText(m_backend, "edit.charset.warn.password") : QString());
			});
			track(m_user, QStringLiteral("credential"));
			track(m_password, QStringLiteral("credential"));
			auto* ul = new QLabel(rjText(m_backend, "edit.cred.username"), m_directBox);
			ul->setBuddy(m_user);
			auto* pl = new QLabel(rjText(m_backend, "edit.cred.password"), m_directBox);
			pl->setBuddy(m_password);
			f->addRow(ul, m_user);
			f->addRow(pl, m_password);
			f->addRow(QString(), m_pwWarn);
			v->addWidget(m_directBox);
		}
		else if (mode == QLatin1String("entry"))
		{
			m_entryBox = new QWidget(w);
			auto* f = new QFormLayout(m_entryBox);
			f->setContentsMargins(24, 0, 0, 0);
			m_entry = new QComboBox(m_entryBox);
			m_entry->setObjectName(QStringLiteral("entryCombo"));
			for (const QJsonValue& e : c.value(QStringLiteral("entries")).toArray())
			{
				const QJsonObject o = e.toObject();
				const QString group = o.value(QStringLiteral("group")).toString();
				m_entry->addItem(o.value(QStringLiteral("title")).toString() + (group.isEmpty() ? QString() : QStringLiteral("  (") + group + QLatin1Char(')')),
				                 o.value(QStringLiteral("uuid")).toString());
			}
			if (c.contains(QStringLiteral("entry")))
				m_entry->setCurrentIndex(qMax(0, m_entry->findData(c.value(QStringLiteral("entry")).toString())));
			track(m_entry, QStringLiteral("credential"));
			auto* el = new QLabel(rjText(m_backend, "edit.cred.entry.label"), m_entryBox);
			el->setBuddy(m_entry);
			f->addRow(el, m_entry);
			v->addWidget(m_entryBox);
		}
		else if (mode == QLatin1String("named"))
		{
			m_namedBox = new QWidget(w);
			auto* f = new QFormLayout(m_namedBox);
			f->setContentsMargins(24, 0, 0, 0);
			m_named = new QComboBox(m_namedBox);
			m_named->setEditable(true);
			for (const QJsonValue& n : c.value(QStringLiteral("names")).toArray())
				m_named->addItem(n.toString(), n.toString());
			m_named->setCurrentText(c.value(QStringLiteral("mode")).toString() == QLatin1String("named") ? c.value(QStringLiteral("name")).toString() : QString());
			track(m_named, QStringLiteral("credential"));
			auto* nl = new QLabel(rjText(m_backend, "edit.cred.named.label"), m_namedBox);
			nl->setBuddy(m_named);
			f->addRow(nl, m_named);
			v->addWidget(m_namedBox);
		}
	}
	const QString mode = c.value(QStringLiteral("mode")).toString(QStringLiteral("inherit"));
	for (auto* b : m_credMode->buttons())
		if (b->property("mode").toString() == mode)
			b->setChecked(true);
	connect(m_credMode, &QButtonGroup::idToggled, this, [this](int, bool on) {
		if (on)
			updateCredentialMode();
	});
	if (!unlocked)
	{
		auto* l = new QLabel(rjText(m_backend, "edit.cred.locked"), w);
		l->setWordWrap(true);
		v->addWidget(l);
		m_credMode->button(2)->setEnabled(false);
		m_credMode->button(3)->setEnabled(false);
	}
	// Woher der wirksame Wert kommt.
	const QJsonObject e = c.value(QStringLiteral("effective")).toObject();
	const QString st = e.value(QStringLiteral("state")).toString();
	QString eff;
	if (st == QLatin1String("found"))
		eff = rjText(m_backend, "edit.cred.effective.found").arg(e.value(QStringLiteral("title")).toString(), e.value(QStringLiteral("name")).toString(), e.value(QStringLiteral("origin")).toString());
	else if (st == QLatin1String("none"))
		eff = rjText(m_backend, "edit.cred.effective.none");
	else if (st == QLatin1String("ambiguous"))
	{
		QStringList names;
		for (const QJsonValue& n : e.value(QStringLiteral("names")).toArray())
			names << n.toString();
		eff = rjText(m_backend, "edit.cred.effective.ambiguous").arg(names.join(QStringLiteral(", ")));
	}
	else
		eff = rjText(m_backend, "edit.cred.effective.missing").arg(e.value(QStringLiteral("name")).toString(QStringLiteral("default")));
	m_credEffective = new QLabel(rjText(m_backend, "edit.cred.effective").arg(eff), w);
	m_credEffective->setObjectName(QStringLiteral("credEffective"));
	m_credEffective->setWordWrap(true);
	m_credEffective->setTextInteractionFlags(Qt::TextSelectableByMouse | Qt::TextSelectableByKeyboard);
	v->addWidget(m_credEffective);
	auto* reset = new QPushButton(rjText(m_backend, "edit.cred.reset"), w);
	connect(reset, &QPushButton::clicked, this, [this] { m_credMode->button(0)->setChecked(true); });
	auto* row = new QHBoxLayout;
	row->addWidget(reset);
	if (m_data.value(QStringLiteral("kind")).toString() != QLatin1String("target"))
	{
		auto* a = new QPushButton(rjText(m_backend, "edit.apply"), w);
		connect(a, &QPushButton::clicked, this, [this] { applyToChildren(QStringLiteral("credentials"), rjText(m_backend, "edit.cat.credentials")); });
		row->addWidget(a);
	}
	row->addStretch(1);
	v->addLayout(row);
	m_errors.insert(QStringLiteral("credential"), errorLabel(w));
	v->addWidget(m_errors.value(QStringLiteral("credential")));
	v->addStretch(1);
	return w;
}

void PropertiesTab::updateCredentialMode()
{
	if (!m_credMode)
		return;
	const QString mode = m_credMode->checkedButton() ? m_credMode->checkedButton()->property("mode").toString() : QString();
	m_directBox->setEnabled(mode == QLatin1String("direct"));
	m_user->setEnabled(mode == QLatin1String("direct"));
	m_password->setEnabled(mode == QLatin1String("direct"));
	m_entryBox->setEnabled(mode == QLatin1String("entry"));
	m_namedBox->setEnabled(mode == QLatin1String("named"));
}

QWidget* PropertiesTab::pageTunnel()
{
	auto* w = new QWidget;
	auto* f = new QFormLayout(w);
	m_tunnel = new QComboBox(w);
	m_tunnel->addItem(rjText(m_backend, "edit.inherit"), QStringLiteral("inherit"));
	m_tunnel->addItem(rjText(m_backend, "edit.none"), QStringLiteral("none"));
	for (const QJsonValue& t : m_data.value(QStringLiteral("tunnels")).toArray())
		m_tunnel->addItem(t.toObject().value(QStringLiteral("name")).toString(), t.toObject().value(QStringLiteral("id")).toString());
	m_tunnel->setCurrentIndex(qMax(0, m_tunnel->findData(m_data.value(QStringLiteral("tunnel")).toString())));
	track(m_tunnel, QStringLiteral("tunnel"));
	auto* l = new QLabel(rjText(m_backend, "edit.tunnel"), w);
	l->setBuddy(m_tunnel);
	f->addRow(l, m_tunnel);
	if (m_data.contains(QStringLiteral("tunnel_effective")))
		f->addRow(QString(), new QLabel(rjText(m_backend, "edit.effective").arg(m_data.value(QStringLiteral("tunnel_effective")).toString()), w));
	if (m_data.value(QStringLiteral("kind")).toString() != QLatin1String("target"))
	{
		auto* a = new QPushButton(rjText(m_backend, "edit.apply"), w);
		connect(a, &QPushButton::clicked, this, [this] { applyToChildren(QStringLiteral("tunnel"), rjText(m_backend, "edit.cat.tunnel")); });
		f->addRow(QString(), a);
	}
	return w;
}

QWidget* PropertiesTab::pageTasks()
{
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	v->addWidget(new QLabel(rjText(m_backend, "edit.tasks.hint"), w));
	m_taskList = new QListWidget(w);
	m_taskList->setAccessibleName(rjText(m_backend, "edit.cat.tasks"));
	auto addItems = [&](const char* key, const QString& kind) {
		for (const QJsonValue& x : m_data.value(QString::fromLatin1(key)).toArray())
		{
			const QJsonObject o = x.toObject();
			auto* it = new QListWidgetItem((kind == QLatin1String("task") ? S("Aufgabe: ") : S("Makro: ")) + o.value(QStringLiteral("name")).toString(), m_taskList);
			it->setFlags(it->flags() | Qt::ItemIsUserCheckable);
			it->setCheckState(o.value(QStringLiteral("linked")).toBool() ? Qt::Checked : Qt::Unchecked);
			it->setData(Qt::UserRole, o.value(QStringLiteral("id")).toString());
			it->setData(Qt::UserRole + 1, kind);
			it->setData(Qt::UserRole + 2, o);
		}
	};
	addItems("tasks", QStringLiteral("task"));
	addItems("macros", QStringLiteral("macro"));
	connect(m_taskList, &QListWidget::itemChanged, this, [this] { markDirty(QStringLiteral("tasks")); });
	v->addWidget(m_taskList, 1);
	auto* f = new QFormLayout;
	m_taskName = new QLineEdit(w);
	m_taskKind = new QLineEdit(w);
	m_taskBody = new QPlainTextEdit(w);
	m_taskBody->setMaximumHeight(120);
	auto* bodyLabel = new QLabel(rjText(m_backend, "edit.tasks.params"), w);
	f->addRow(rjText(m_backend, "edit.tasks.name"), m_taskName);
	f->addRow(rjText(m_backend, "edit.tasks.kind"), m_taskKind);
	f->addRow(bodyLabel, m_taskBody);
	v->addLayout(f);
	auto* apply = new QPushButton(rjText(m_backend, "edit.tasks.apply"), w);
	v->addWidget(apply);
	connect(m_taskList, &QListWidget::currentItemChanged, this, [this, bodyLabel](QListWidgetItem* it) {
		if (!it)
			return;
		const QJsonObject o = it->data(Qt::UserRole + 2).toJsonObject();
		const bool task = it->data(Qt::UserRole + 1).toString() == QLatin1String("task");
		m_taskName->setText(o.value(QStringLiteral("name")).toString());
		m_taskKind->setText(o.value(QStringLiteral("kind")).toString());
		m_taskKind->setEnabled(task);
		bodyLabel->setText(rjText(m_backend, task ? "edit.tasks.params" : "edit.tasks.steps"));
		if (task)
		{
			QStringList lines;
			const QJsonObject p = o.value(QStringLiteral("params")).toObject();
			for (auto i = p.begin(); i != p.end(); ++i)
				lines << i.key() + QLatin1Char('=') + i.value().toString();
			m_taskBody->setPlainText(lines.join(QLatin1Char('\n')));
		}
		else
			m_taskBody->setPlainText(QString::fromUtf8(QJsonDocument(o.value(QStringLiteral("steps")).toArray()).toJson(QJsonDocument::Indented)));
	});
	connect(apply, &QPushButton::clicked, this, [this] {
		QListWidgetItem* it = m_taskList->currentItem();
		if (!it)
			return;
		const bool task = it->data(Qt::UserRole + 1).toString() == QLatin1String("task");
		QJsonObject cmd{ { QStringLiteral("op"), task ? QStringLiteral("update_task") : QStringLiteral("update_macro") },
			             { QStringLiteral("id"), it->data(Qt::UserRole).toString() },
			             { QStringLiteral("name"), m_taskName->text() } };
		if (task)
		{
			cmd.insert(QStringLiteral("kind"), m_taskKind->text());
			QJsonObject params;
			for (const QString& line : m_taskBody->toPlainText().split(QLatin1Char('\n'), Qt::SkipEmptyParts))
			{
				const int eq = line.indexOf(QLatin1Char('='));
				if (eq > 0)
					params.insert(line.left(eq), line.mid(eq + 1));
			}
			cmd.insert(QStringLiteral("params"), params);
		}
		else
			cmd.insert(QStringLiteral("steps"), m_taskBody->toPlainText());
		const QJsonObject r = parse(edit(cmd));
		if (r.contains(QStringLiteral("error")))
			m_status->setText(rjText(m_backend, "edit.error").arg(r.value(QStringLiteral("error")).toString()));
		else
		{
			emit indexChanged();
			reload();
			selectCategory(QStringLiteral("tasks"));
		}
	});
	return w;
}

QWidget* PropertiesTab::pageClipboard()
{
	auto* w = new QWidget;
	auto* f = new QFormLayout(w);
	const bool target = m_data.value(QStringLiteral("kind")).toString() == QLatin1String("target");
	const QString proto = m_data.value(QStringLiteral("protocol")).toString();
	const QJsonObject o = m_data.value(QStringLiteral("options")).toObject();
	m_clip = new QComboBox(w);
	if (!target)
		m_clip->addItem(rjText(m_backend, "edit.inherit") + QStringLiteral(" (") +
		                  m_backend->text(QStringLiteral("edit.clip.") + m_data.value(QStringLiteral("clipboard_effective")).toString()) + QLatin1Char(')'),
		                QStringLiteral("inherit"));
	for (const char* c : kClip)
		m_clip->addItem(m_backend->text(QStringLiteral("edit.clip.") + QString::fromLatin1(c)), QString::fromLatin1(c));
	auto* cl = new QLabel(rjText(m_backend, "edit.clip.direction"), w);
	cl->setBuddy(m_clip);
	if (target)
	{
		if (proto == QLatin1String("ssh"))
		{
			delete m_clip;
			delete cl;
			m_clip = nullptr;
			auto* wr = new QCheckBox(rjText(m_backend, "edit.clip.osc52_write"), w);
			wr->setChecked(o.value(QStringLiteral("osc52_write")).toBool(true));
			auto* rd = new QCheckBox(rjText(m_backend, "edit.clip.osc52_read"), w);
			rd->setChecked(o.value(QStringLiteral("osc52_read")).toBool(false));
			track(wr, QStringLiteral("options"));
			track(rd, QStringLiteral("options"));
			m_opts.insert(QStringLiteral("osc52_write"), wr);
			m_opts.insert(QStringLiteral("osc52_read"), rd);
			f->addRow(QString(), wr);
			f->addRow(QString(), rd);
			return w;
		}
		if (o.contains(QStringLiteral("clipboard")) || proto == QLatin1String("rdp") || proto == QLatin1String("vnc") || proto == QLatin1String("hyperv"))
		{
			m_clip->setCurrentIndex(qMax(0, m_clip->findData(o.value(QStringLiteral("clipboard")).toString(QStringLiteral("both")))));
			track(m_clip, QStringLiteral("options"));
			f->addRow(cl, m_clip);
			if (proto == QLatin1String("rdp") || proto == QLatin1String("hyperv"))
			{
				const QJsonValue lim = o.value(QStringLiteral("clipboard_limit"));
				auto* e = new QLineEdit(lim.isDouble() ? QString::number(lim.toInteger()) : QString(), w);
				e->setProperty("numeric", true);
				track(e, QStringLiteral("options"));
				m_opts.insert(QStringLiteral("clipboard_limit"), e);
				f->addRow(rjText(m_backend, "edit.clip.limit"), e);
				m_errors.insert(QStringLiteral("clipboard_limit"), errorLabel(w));
				f->addRow(QString(), m_errors.value(QStringLiteral("clipboard_limit")));
			}
		}
		else
		{
			delete m_clip;
			delete cl;
			m_clip = nullptr;
			f->addRow(new QLabel(QStringLiteral("–"), w));
		}
		return w;
	}
	m_clip->setCurrentIndex(qMax(0, m_clip->findData(m_data.value(QStringLiteral("clipboard")).toString())));
	track(m_clip, QStringLiteral("clipboard"));
	f->addRow(cl, m_clip);
	auto* a = new QPushButton(rjText(m_backend, "edit.apply"), w);
	connect(a, &QPushButton::clicked, this, [this] { applyToChildren(QStringLiteral("clipboard"), rjText(m_backend, "edit.cat.clipboard")); });
	f->addRow(QString(), a);
	return w;
}

QWidget* PropertiesTab::pageOrganisation()
{
	auto* w = new QWidget;
	auto* v = new QVBoxLayout(w);
	auto* nl = new QLabel(rjText(m_backend, "edit.notes"), w);
	m_notes = new QPlainTextEdit(m_data.value(QStringLiteral("notes")).toString(), w);
	m_notes->setTabChangesFocus(true);
	nl->setBuddy(m_notes);
	track(m_notes, QStringLiteral("notes"));
	v->addWidget(nl);
	v->addWidget(m_notes, 1);
	v->addWidget(new QLabel(rjText(m_backend, "edit.fields"), w));
	m_fields = new QTableWidget(0, 2, w);
	m_fields->setHorizontalHeaderLabels({ rjText(m_backend, "edit.fields.name"), rjText(m_backend, "edit.fields.value") });
	m_fields->horizontalHeader()->setStretchLastSection(true);
	m_fields->setTabKeyNavigation(false);
	const QJsonObject fields = m_data.value(QStringLiteral("fields")).toObject();
	for (auto i = fields.begin(); i != fields.end(); ++i)
	{
		const int r = m_fields->rowCount();
		m_fields->insertRow(r);
		m_fields->setItem(r, 0, new QTableWidgetItem(i.key()));
		m_fields->setItem(r, 1, new QTableWidgetItem(i.value().toString()));
	}
	track(m_fields, QStringLiteral("fields"));
	v->addWidget(m_fields, 1);
	auto* row = new QHBoxLayout;
	auto* add = new QPushButton(rjText(m_backend, "edit.fields.add"), w);
	auto* rem = new QPushButton(rjText(m_backend, "edit.fields.remove"), w);
	connect(add, &QPushButton::clicked, this, [this] {
		const int r = m_fields->rowCount();
		m_fields->insertRow(r);
		m_fields->setItem(r, 0, new QTableWidgetItem);
		m_fields->setItem(r, 1, new QTableWidgetItem);
		m_fields->setCurrentCell(r, 0);
		m_fields->editItem(m_fields->item(r, 0));
		markDirty(QStringLiteral("fields"));
	});
	connect(rem, &QPushButton::clicked, this, [this] {
		if (m_fields->currentRow() >= 0)
		{
			m_fields->removeRow(m_fields->currentRow());
			markDirty(QStringLiteral("fields"));
		}
	});
	row->addWidget(add);
	row->addWidget(rem);
	row->addStretch(1);
	v->addLayout(row);
	return w;
}

QWidget* PropertiesTab::pageParent()
{
	auto* w = new QWidget;
	auto* f = new QFormLayout(w);
	m_parent = new QComboBox(w);
	for (const QJsonValue& x : m_data.value(QStringLiteral("folders")).toArray())
		m_parent->addItem(x.toObject().value(QStringLiteral("path")).toString(), x.toObject().value(QStringLiteral("id")).toString());
	m_parent->setCurrentIndex(qMax(0, m_parent->findData(m_data.value(QStringLiteral("parent")).toString())));
	track(m_parent, QStringLiteral("parent"));
	auto* l = new QLabel(rjText(m_backend, "edit.parent"), w);
	l->setBuddy(m_parent);
	f->addRow(l, m_parent);
	return w;
}

QJsonObject PropertiesTab::collect() const
{
	QJsonObject f;
	const bool target = m_data.value(QStringLiteral("kind")).toString() == QLatin1String("target");
	if (m_dirty.contains(QStringLiteral("name")))
		f.insert(QStringLiteral("name"), m_name->text());
	if (m_dirty.contains(QStringLiteral("description")))
		f.insert(QStringLiteral("description"), m_desc->text());
	if (target && m_dirty.contains(QStringLiteral("host")))
		f.insert(QStringLiteral("host"), m_host->text());
	if (target && m_dirty.contains(QStringLiteral("port")))
		f.insert(QStringLiteral("port"), m_port->text().isEmpty() ? QJsonValue() : QJsonValue(m_port->text()));
	if (target && m_dirty.contains(QStringLiteral("options")))
	{
		// Unbekannte Optionen bleiben erhalten.
		QJsonObject o = m_data.value(QStringLiteral("options")).toObject();
		for (auto i = m_opts.begin(); i != m_opts.end(); ++i)
		{
			if (auto* e = qobject_cast<QLineEdit*>(i.value()))
			{
				if (e->text().isEmpty())
					o.remove(i.key());
				else if (e->property("numeric").toBool())
				{
					bool ok = false;
					const qint64 n = e->text().toLongLong(&ok);
					o.insert(i.key(), ok ? QJsonValue(n) : QJsonValue(e->text()));
				}
				else
					o.insert(i.key(), e->text());
			}
			else if (auto* c = qobject_cast<QCheckBox*>(i.value()))
				o.insert(i.key(), c->isChecked());
		}
		if (m_clip)
			o.insert(QStringLiteral("clipboard"), m_clip->currentData().toString());
		f.insert(QStringLiteral("options"), o);
	}
	if (!target && m_clip && m_dirty.contains(QStringLiteral("clipboard")))
		f.insert(QStringLiteral("clipboard"), m_clip->currentData().toString());
	if (m_dirty.contains(QStringLiteral("tunnel")))
		f.insert(QStringLiteral("tunnel"), m_tunnel->currentData().toString());
	if (m_dirty.contains(QStringLiteral("snmp")))
		f.insert(QStringLiteral("snmp"), m_snmp->currentData().toString());
	if (m_dirty.contains(QStringLiteral("psrp")))
	{
		const QString p = m_psrp->currentData().toString();
		if (p == QLatin1String("inherit"))
			f.insert(QStringLiteral("psrp_transport"), QStringLiteral("inherit"));
		else
		{
			QJsonObject t;
			if (p == QLatin1String("ssh"))
				t.insert(QStringLiteral("type"), QStringLiteral("ssh"));
			else
			{
				t.insert(QStringLiteral("type"), QStringLiteral("winrm"));
				t.insert(QStringLiteral("https"), p == QLatin1String("winrm_https"));
				t.insert(QStringLiteral("auth"), m_psrpAuth->currentData().toString());
				t.insert(QStringLiteral("verify_certificate"), m_psrpVerify->isChecked());
			}
			bool ok = false;
			const int port = m_psrpPort->text().toInt(&ok);
			if (ok)
				t.insert(QStringLiteral("port"), port);
			f.insert(QStringLiteral("psrp_transport"), t);
		}
	}
	if (m_dirty.contains(QStringLiteral("notes")))
		f.insert(QStringLiteral("notes"), m_notes->toPlainText());
	if (m_dirty.contains(QStringLiteral("fields")))
	{
		QJsonObject fields;
		for (int r = 0; r < m_fields->rowCount(); ++r)
			if (m_fields->item(r, 0) && !m_fields->item(r, 0)->text().isEmpty())
				fields.insert(m_fields->item(r, 0)->text(), m_fields->item(r, 1) ? m_fields->item(r, 1)->text() : QString());
		f.insert(QStringLiteral("fields"), fields);
	}
	if (m_dirty.contains(QStringLiteral("tasks")))
	{
		QJsonArray tasks, macros;
		for (int i = 0; i < m_taskList->count(); ++i)
		{
			QListWidgetItem* it = m_taskList->item(i);
			if (it->checkState() != Qt::Checked)
				continue;
			(it->data(Qt::UserRole + 1).toString() == QLatin1String("task") ? tasks : macros).append(it->data(Qt::UserRole).toString());
		}
		f.insert(QStringLiteral("tasks"), tasks);
		f.insert(QStringLiteral("macros"), macros);
	}
	if (m_parent && m_dirty.contains(QStringLiteral("parent")))
		f.insert(QStringLiteral("parent"), m_parent->currentData().toString());
	QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("update") }, { QStringLiteral("id"), m_id }, { QStringLiteral("fields"), f }, { QStringLiteral("save"), true } };
	if (m_dirty.contains(QStringLiteral("credential")) && m_credMode->checkedButton())
	{
		const QString mode = m_credMode->checkedButton()->property("mode").toString();
		QJsonObject c{ { QStringLiteral("mode"), mode } };
		if (mode == QLatin1String("direct"))
		{
			c.insert(QStringLiteral("username"), m_user->text());
			// Kennwort unverändert (kein Trimmen); leer heißt „unverändert lassen“.
			if (!m_password->text().isEmpty())
				c.insert(QStringLiteral("password"), m_password->text());
		}
		else if (mode == QLatin1String("entry"))
			c.insert(QStringLiteral("entry"), m_entry->currentData().toString());
		else if (mode == QLatin1String("named"))
			c.insert(QStringLiteral("name"), m_named->currentText());
		cmd.insert(QStringLiteral("credential"), c);
	}
	return cmd;
}

void PropertiesTab::showErrors(const QJsonObject& errors)
{
	for (auto* l : std::as_const(m_errors))
		setWarn(l, QString());
	QString first;
	for (auto i = errors.begin(); i != errors.end(); ++i)
	{
		if (auto* l = m_errors.value(i.key(), nullptr))
			setWarn(l, i.value().toString());
		if (first.isEmpty())
			first = i.key();
	}
	if (first == QLatin1String("name"))
		selectCategory(QStringLiteral("general"));
	else if (first == QLatin1String("clipboard_limit"))
		selectCategory(QStringLiteral("clipboard"));
	else if (!first.isEmpty())
		selectCategory(QStringLiteral("connection"));
	m_status->setText(rjText(m_backend, "edit.error").arg(errors.value(first).toString()));
}

void PropertiesTab::save()
{
	if (!isDirty())
		return;
	// Favorit ist eine eigene Aktion (gilt sofort).
	if (m_dirty.contains(QStringLiteral("favorite")))
		edit({ { QStringLiteral("op"), QStringLiteral("favorite") }, { QStringLiteral("id"), m_id }, { QStringLiteral("on"), m_fav->isChecked() } });
	QJsonObject cmd = collect();
	QString json = QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact));
	const QString res = m_backend->edit(json);
	// Puffer mit dem Kennwort überschreiben.
	json.fill(QLatin1Char(' '));
	if (m_password)
	{
		QString pw = m_password->text();
		pw.fill(QLatin1Char(' '));
		m_password->clear();
	}
	handleSaveResult(parse(res));
}

void PropertiesTab::handleSaveResult(const QJsonObject& r)
{
	if (r.contains(QStringLiteral("errors")))
	{
		showErrors(r.value(QStringLiteral("errors")).toObject());
		return;
	}
	if (r.contains(QStringLiteral("conflict")))
	{
		const QJsonObject c = r.value(QStringLiteral("conflict")).toObject();
		emit indexChanged();
		m_bar->ask(rjText(m_backend, "bar.conflict").arg(c.value(QStringLiteral("mine")).toInteger()).arg(c.value(QStringLiteral("theirs")).toInteger()),
		           { rjText(m_backend, "bar.conflict.mine"), rjText(m_backend, "bar.conflict.theirs"), rjText(m_backend, "bar.conflict.both") },
		           [this](int i) {
			           if (i < 0)
				           return;
			           const char* choice[] = { "mine", "theirs", "both" };
			           handleSaveResult(parse(edit({ { QStringLiteral("op"), QStringLiteral("save") }, { QStringLiteral("choice"), QString::fromLatin1(choice[i]) } })));
		           });
		return;
	}
	if (r.contains(QStringLiteral("error")))
	{
		m_status->setText(rjText(m_backend, "edit.error").arg(r.value(QStringLiteral("error")).toString()));
		emit indexChanged();
		return;
	}
	reload();
	emit indexChanged();
	const QString msg = r.value(QStringLiteral("notice")).toString(rjText(m_backend, "status.saved"));
	m_status->setText(msg);
	emit status(msg);
}

void PropertiesTab::applyToChildren(const QString& kind, const QString& label)
{
	if (isDirty())
	{
		m_status->setText(rjText(m_backend, "status.save_first"));
		return;
	}
	const QJsonObject p = parse(edit({ { QStringLiteral("op"), QStringLiteral("apply_preview") }, { QStringLiteral("id"), m_id }, { QStringLiteral("kind"), kind } }));
	const int n = p.value(QStringLiteral("count")).toInt();
	if (n == 0)
	{
		m_bar->ask(rjText(m_backend, "bar.apply.none"), {}, nullptr);
		return;
	}
	m_bar->ask(rjText(m_backend, "bar.apply").arg(label).arg(n), { rjText(m_backend, "bar.apply.button") }, [this, kind](int i) {
		if (i != 0)
			return;
		const QJsonObject r = parse(edit({ { QStringLiteral("op"), QStringLiteral("apply") }, { QStringLiteral("id"), m_id }, { QStringLiteral("kind"), kind } }));
		m_status->setText(r.contains(QStringLiteral("error")) ? r.value(QStringLiteral("error")).toString()
		                                                      : rjText(m_backend, "status.applied").arg(r.value(QStringLiteral("count")).toInt()));
		emit indexChanged();
	});
}

void PropertiesTab::saveTemplate()
{
	const QString proto = m_data.value(QStringLiteral("protocol")).toString().toUpper();
	m_bar->prompt(rjText(m_backend, "bar.template"), m_data.value(QStringLiteral("name")).toString(), [this, proto](const QString& name) {
		m_bar->choose(rjText(m_backend, "bar.template.default").arg(proto) + QLatin1Char('?'),
		              { { S("Nein"), QStringLiteral("0") }, { S("Ja"), QStringLiteral("1") } }, [this, name](const QString& def) {
			              const QJsonObject r = parse(edit({ { QStringLiteral("op"), QStringLiteral("template_save") },
			                                                 { QStringLiteral("id"), m_id },
			                                                 { QStringLiteral("name"), name },
			                                                 { QStringLiteral("default"), def == QLatin1String("1") } }));
			              m_status->setText(r.contains(QStringLiteral("error")) ? r.value(QStringLiteral("error")).toString() : S("Vorlage gespeichert"));
		              });
	});
}

void PropertiesTab::requestClose(std::function<void()> close)
{
	if (!isDirty())
	{
		close();
		return;
	}
	m_bar->ask(rjText(m_backend, "bar.unsaved").arg(m_data.value(QStringLiteral("name")).toString()),
	           { rjText(m_backend, "bar.save"), rjText(m_backend, "bar.discard") }, [this, close](int i) {
		           if (i == 0)
		           {
			           save();
			           if (!isDirty())
				           close();
		           }
		           else if (i == 1)
			           close();
	           });
}

// ------------------------------------------------------------ UnlockOverlay

UnlockOverlay::UnlockOverlay(RjBackend* backend, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
{
	setObjectName(QStringLiteral("unlockOverlay"));
	setAttribute(Qt::WA_StyledBackground, true);
	setAutoFillBackground(false);
	auto* outer = new QVBoxLayout(this);
	auto* box = new QFrame(this);
	box->setObjectName(QStringLiteral("unlockBox"));
	box->setFrameShape(QFrame::StyledPanel);
	box->setMaximumWidth(560);
	box->setAutoFillBackground(true);
	outer->addStretch(1);
	auto* row = new QHBoxLayout;
	row->addStretch(1);
	row->addWidget(box, 3);
	row->addStretch(1);
	outer->addLayout(row);
	outer->addStretch(1);
	auto* v = new QVBoxLayout(box);
	auto* title = new QLabel(rjText(backend, "unlock.title"), box);
	QFont tf = title->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.4);
	tf.setBold(true);
	title->setFont(tf);
	v->addWidget(title);
	auto* f = new QFormLayout;
	auto* fs = new QFileSystemModel(this);
	fs->setRootPath(QString());
	m_path = new QLineEdit(box);
	m_path->setCompleter(new QCompleter(fs, this));
	m_password = new QLineEdit(box);
	m_password->setEchoMode(QLineEdit::Password);
	m_keyFile = new QLineEdit(box);
	m_keyFile->setCompleter(new QCompleter(fs, this));
	auto add = [&](const char* label, QLineEdit* e) {
		auto* l = new QLabel(rjText(backend, label), box);
		l->setBuddy(e);
		f->addRow(l, e);
	};
	add("unlock.file", m_path);
	add("unlock.password", m_password);
	add("unlock.keyfile", m_keyFile);
	v->addLayout(f);
	m_message = new QLabel(box);
	m_message->setWordWrap(true);
	v->addWidget(m_message);
	auto* buttons = new QHBoxLayout;
	buttons->addStretch(1);
	m_cancel = new QPushButton(rjText(backend, "unlock.cancel"), box);
	m_ok = new QPushButton(rjText(backend, "unlock.button"), box);
	m_ok->setDefault(true);
	buttons->addWidget(m_cancel);
	buttons->addWidget(m_ok);
	v->addLayout(buttons);
	connect(m_ok, &QPushButton::clicked, this, &UnlockOverlay::submit);
	connect(m_password, &QLineEdit::returnPressed, this, &UnlockOverlay::submit);
	connect(m_cancel, &QPushButton::clicked, this, &QWidget::hide);
	if (parent)
		parent->installEventFilter(this);
	hide();
}

void UnlockOverlay::open(const QString& path, bool locked)
{
	if (!path.isEmpty())
		m_path->setText(path);
	m_cancel->setVisible(!locked);
	m_message->clear();
	setBusy(false);
	setGeometry(parentWidget()->rect());
	raise();
	show();
	(m_path->text().isEmpty() ? m_path : m_password)->setFocus();
}

void UnlockOverlay::setBusy(bool busy, const QString& message)
{
	m_ok->setEnabled(!busy);
	m_password->setEnabled(!busy);
	if (!message.isEmpty() || !busy)
		m_message->setText(message);
}

void UnlockOverlay::submit()
{
	QString pw = m_password->text();
	setBusy(true, rjText(m_backend, "unlock.busy"));
	emit unlockRequested(m_path->text(), pw, m_keyFile->text());
	pw.fill(QLatin1Char(' '));
	m_password->clear();
}

void UnlockOverlay::paintEvent(QPaintEvent*)
{
	// Deckt alles ab, auch Bilder laufender Sitzungen.
	QPainter p(this);
	p.fillRect(rect(), palette().window());
}

bool UnlockOverlay::eventFilter(QObject* o, QEvent* e)
{
	if (o == parentWidget() && e->type() == QEvent::Resize && isVisible())
		setGeometry(parentWidget()->rect());
	return QWidget::eventFilter(o, e);
}
