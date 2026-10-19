#include "editor.h"
#include "views.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QDir>
#include <QtCore/QFileInfo>
#include <QtCore/QStandardPaths>
#include <QtCore/QRegularExpression>
#include <QtWidgets/QMenu>
#include <QtCore/QTimer>
#include <QtCore/QElapsedTimer>
#include <QtWidgets/QProgressBar>
#include <QtWidgets/QStyle>
#include <QtGui/QDesktopServices>
#include <QtGui/QIcon>
#include <QtGui/QKeyEvent>
#include <QtGui/QPainter>
#include <QtGui/QPainterPath>
#include <QtGui/QShortcut>
#include <QtGui/QFileSystemModel>
#include <QtGui/QFontDatabase>
#include <QtWidgets/QButtonGroup>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QCompleter>
#include <QtWidgets/QFileDialog>
#include <QtWidgets/QFontComboBox>
#include <QtWidgets/QSpinBox>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QGroupBox>
#include <QtCore/QProcess>
#include <QtCore/QProcessEnvironment>
#include <QtWidgets/QApplication>
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
#include <QtWidgets/QToolButton>
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
		emit cancelled();
	});
	lay->addWidget(m_cancel);
	connect(m_line, &QLineEdit::returnPressed, m_buttons[0], &QPushButton::click);
	hide();
}

void ActionBar::reset(const QString& text)
{
	m_cancel->setText(rjText(m_backend, "bar.cancel"));
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

void ActionBar::inform(const QString& text)
{
	reset(text);
	m_done = nullptr;
	m_cancel->setText(rjText(m_backend, "bar.close"));
	m_cancel->setFocus();
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

void PropertiesTab::setDetectedFields(const QJsonObject& e)
{
	if (!m_detectButton)
		return;
	m_detectButton->setEnabled(true);
	const QString error = e.value(QStringLiteral("error")).toString();
	if (!error.isEmpty())
	{
		m_detectStatus->setText(rjText(m_backend, "edit.web.detect.error").arg(error));
		return;
	}
	const QJsonObject f = e.value(QStringLiteral("fields")).toObject();
	const auto load = [&f](const char* key) {
		QList<QPair<QString, QString>> list;
		for (const QJsonValue& v : f.value(QLatin1String(key)).toArray())
			list.append({ v.toObject().value(QStringLiteral("label")).toString(), v.toObject().value(QStringLiteral("selector")).toString() });
		return list;
	};
	m_candidates[QStringLiteral("user_selector")] = load("users");
	m_candidates[QStringLiteral("password_selector")] = load("passwords");
	m_candidates[QStringLiteral("submit_selector")] = load("submits");
	const int u = m_candidates.value(QStringLiteral("user_selector")).size(), p = m_candidates.value(QStringLiteral("password_selector")).size(), s = m_candidates.value(QStringLiteral("submit_selector")).size();
	m_detectStatus->setText(u + p + s == 0 ? rjText(m_backend, "edit.web.detect.none") : rjText(m_backend, "edit.web.detect.found").arg(u).arg(p).arg(s));
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
	m_akEnabled = nullptr;
	m_akScript = nullptr;
	m_akTrigger = nullptr;
	m_akWait = nullptr;
	m_akMax = nullptr;
	m_akReconnect = nullptr;
	m_akUser = nullptr;
	m_akPass = nullptr;
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
	connect(saveB, &QPushButton::clicked, this, &PropertiesTab::saveAndClose);
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
		if (proto == QLatin1String("web"))
		{
			// Bei Web genügt die Adresse (URL); Host und Port werden daraus abgeleitet.
			for (QWidget* x : { static_cast<QWidget*>(m_host), static_cast<QWidget*>(m_hostWarn), static_cast<QWidget*>(m_errors.value(QStringLiteral("host"))),
			                    static_cast<QWidget*>(m_port), static_cast<QWidget*>(m_errors.value(QStringLiteral("port"))) })
				f->setRowVisible(x, false);
		}
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
			if (proto == QLatin1String("rdp"))
			{
				auto* g = new QComboBox(w);
				g->addItem(rjText(m_backend, "edit.opt.gfx.auto"), QStringLiteral("auto"));
				g->addItem(rjText(m_backend, "edit.opt.gfx.on"), QStringLiteral("on"));
				g->addItem(rjText(m_backend, "edit.opt.gfx.off"), QStringLiteral("off"));
				g->setCurrentIndex(qMax(0, g->findData(o.value(QStringLiteral("gfx")).toString(QStringLiteral("auto")))));
				track(g, QStringLiteral("options"));
				f->addRow(rjText(m_backend, "edit.opt.gfx"), g);
				m_opts.insert(QStringLiteral("gfx"), g);
			}
		}
		else if (proto == QLatin1String("ssh"))
		{
			text("term", "edit.opt.term");
			text("keepalive_secs", "edit.opt.keepalive_secs");
			check("legacy_algorithms", "edit.opt.legacy_algorithms");
		}
		// Tastenfolge nach dem Verbinden: RDP (nicht Hyper-V-Konsole) und SSH.
		if (proto == QLatin1String("rdp") || proto == QLatin1String("ssh"))
		{
			const QJsonObject ak = o.value(QStringLiteral("auto_keys")).toObject();
			auto* head = new QLabel(rjText(m_backend, "edit.autokeys"), w);
			head->setObjectName(QStringLiteral("heading"));
			f->addRow(head);
			m_akEnabled = new QCheckBox(rjText(m_backend, "edit.autokeys.enabled"), w);
			m_akEnabled->setChecked(ak.value(QStringLiteral("enabled")).toBool());
			track(m_akEnabled, QStringLiteral("options"));
			f->addRow(QString(), m_akEnabled);
			m_akScript = new QPlainTextEdit(ak.value(QStringLiteral("script")).toString(), w);
			m_akScript->setPlaceholderText(rjText(m_backend, "edit.autokeys.placeholder"));
			m_akScript->setTabChangesFocus(true);
			m_akScript->setMinimumHeight(110);
			m_akScript->setAccessibleName(rjText(m_backend, "edit.autokeys.script"));
			track(m_akScript, QStringLiteral("options"));
			f->addRow(rjText(m_backend, "edit.autokeys.script"), m_akScript);
			auto* example = new QPushButton(rjText(m_backend, "edit.autokeys.example"), w);
			connect(example, &QPushButton::clicked, this, [this] {
				const QString ex = rjText(m_backend, "edit.autokeys.placeholder");
				if (!m_akScript->toPlainText().trimmed().isEmpty())
					m_akScript->appendPlainText(QString());
				m_akScript->appendPlainText(ex);
				m_akScript->setFocus();
			});
			f->addRow(QString(), example);
			auto* help = new QLabel(rjText(m_backend, "edit.autokeys.help"), w);
			help->setWordWrap(true);
			help->setTextInteractionFlags(Qt::TextSelectableByMouse);
			f->addRow(QString(), help);
			// Zweites Kennwort (z. B. Anmeldung an der Sitzung nach dem RDP-Kennwort): im Skript {cred:Sitzung:password}.
			{
				const QJsonObject aux = m_data.value(QStringLiteral("aux_credential")).toObject();
				auto* ah = new QLabel(rjText(m_backend, "edit.autokeys.aux"), w);
				ah->setObjectName(QStringLiteral("heading"));
				f->addRow(ah);
				m_akUser = new QLineEdit(aux.value(QStringLiteral("username")).toString(), w);
				m_akUser->setAccessibleName(rjText(m_backend, "edit.autokeys.aux.user"));
				track(m_akUser, QStringLiteral("aux"));
				f->addRow(rjText(m_backend, "edit.autokeys.aux.user"), m_akUser);
				m_akPass = new QLineEdit(w);
				m_akPass->setEchoMode(QLineEdit::Password);
				m_akPass->setAccessibleName(rjText(m_backend, "edit.autokeys.aux.password"));
				m_akPass->setPlaceholderText(aux.value(QStringLiteral("exists")).toBool() ? rjText(m_backend, "edit.cred.password.keep") : QString());
				track(m_akPass, QStringLiteral("aux"));
				f->addRow(rjText(m_backend, "edit.autokeys.aux.password"), m_akPass);
				auto* auxHelp = new QLabel(rjText(m_backend, "edit.autokeys.aux.help"), w);
				auxHelp->setWordWrap(true);
				auxHelp->setTextInteractionFlags(Qt::TextSelectableByMouse);
				f->addRow(QString(), auxHelp);
			}
			m_akTrigger = new QComboBox(w);
			m_akTrigger->addItem(rjText(m_backend, "edit.autokeys.trigger.quiet"), QStringLiteral("quiet"));
			m_akTrigger->addItem(rjText(m_backend, "edit.autokeys.trigger.delay"), QStringLiteral("delay"));
			m_akTrigger->setCurrentIndex(qMax(0, m_akTrigger->findData(ak.value(QStringLiteral("trigger")).toString(QStringLiteral("quiet")))));
			track(m_akTrigger, QStringLiteral("options"));
			f->addRow(rjText(m_backend, "edit.autokeys.trigger"), m_akTrigger);
			m_akWait = new QLineEdit(QString::number(ak.value(QStringLiteral("wait_ms")).toInt(3000)), w);
			m_akWait->setAccessibleName(rjText(m_backend, "edit.autokeys.wait"));
			track(m_akWait, QStringLiteral("options"));
			f->addRow(rjText(m_backend, "edit.autokeys.wait"), m_akWait);
			m_akMax = new QLineEdit(QString::number(ak.value(QStringLiteral("max_ms")).toInt(15000)), w);
			m_akMax->setAccessibleName(rjText(m_backend, "edit.autokeys.max"));
			track(m_akMax, QStringLiteral("options"));
			f->addRow(rjText(m_backend, "edit.autokeys.max"), m_akMax);
			// „Spätestens nach“ gilt nur beim Auslöser „ruhig“.
			auto syncMax = [this] { m_akMax->setEnabled(m_akTrigger->currentData().toString() == QLatin1String("quiet")); };
			connect(m_akTrigger, &QComboBox::currentIndexChanged, this, syncMax);
			syncMax();
			m_akReconnect = new QComboBox(w);
			m_akReconnect->addItem(rjText(m_backend, "edit.autokeys.reconnect.always"), QStringLiteral("always"));
			m_akReconnect->addItem(rjText(m_backend, "edit.autokeys.reconnect.first_only"), QStringLiteral("first_only"));
			m_akReconnect->addItem(rjText(m_backend, "edit.autokeys.reconnect.manual_only"), QStringLiteral("manual_only"));
			m_akReconnect->setCurrentIndex(qMax(0, m_akReconnect->findData(ak.value(QStringLiteral("on_reconnect")).toString(QStringLiteral("always")))));
			track(m_akReconnect, QStringLiteral("options"));
			f->addRow(rjText(m_backend, "edit.autokeys.reconnect"), m_akReconnect);
			m_errors.insert(QStringLiteral("auto_keys"), errorLabel(w));
			f->addRow(QString(), m_errors.value(QStringLiteral("auto_keys")));
		}
		else if (proto == QLatin1String("vnc"))
			check("view_only", "edit.opt.view_only");
		else if (proto == QLatin1String("web") || proto == QLatin1String("proxmox"))
		{
			text("url", "edit.opt.url");
			// Ziele, die bisher nur einen Host haben: daraus ergibt sich der Vorschlag für die Adresse.
			if (proto == QLatin1String("web"))
				if (auto* ue = qobject_cast<QLineEdit*>(m_opts.value(QStringLiteral("url"))); ue && ue->text().isEmpty())
					ue->setPlaceholderText(m_data.value(QStringLiteral("default_url")).toString().isEmpty() ? QStringLiteral("https://rechner/") : m_data.value(QStringLiteral("default_url")).toString());
			check("ignore_certificate", "edit.opt.ignore_certificate");
			if (proto == QLatin1String("web"))
			{
				auto* help = new QLabel(rjText(m_backend, "edit.web.help"), w);
				help->setWordWrap(true);
				f->addRow(QString(), help);
			}
			text("user_selector", "edit.opt.user_selector");
			text("password_selector", "edit.opt.password_selector");
			text("submit_selector", "edit.opt.submit_selector");
			for (const char* key : { "user_selector", "password_selector", "submit_selector" })
			{
				auto* e = qobject_cast<QLineEdit*>(m_opts.value(QString::fromLatin1(key)));
				if (!e)
					continue;
				e->setPlaceholderText(rjText(m_backend, "edit.web.auto"));
				// Pfeil im Feld: gefundene Elemente zur Auswahl (nach „Felder erkennen“).
				QAction* arrow = e->addAction(style()->standardIcon(QStyle::SP_ArrowDown), QLineEdit::TrailingPosition);
				arrow->setToolTip(rjText(m_backend, "edit.web.detect"));
				const QString k = QString::fromLatin1(key);
				connect(arrow, &QAction::triggered, this, [this, e, k] {
					QMenu menu(e);
					// Auswahl im Menü gilt wie eine Eingabe (sonst übernimmt „Speichern“ sie nicht).
					menu.addAction(rjText(m_backend, "edit.web.dropdown.auto"), [this, e] { e->clear(); markDirty(QStringLiteral("options")); });
					menu.addSeparator();
					const auto items = m_candidates.value(k);
					if (items.isEmpty())
						menu.addAction(rjText(m_backend, "edit.web.dropdown.empty"))->setEnabled(false);
					for (const auto& c : items)
					{
						QString label = c.first == c.second ? c.second : QStringLiteral("%1   (%2)").arg(c.first, c.second);
						menu.addAction(label, [this, e, sel = c.second] { e->setText(sel); markDirty(QStringLiteral("options")); });
					}
					menu.exec(e->mapToGlobal(QPoint(0, e->height())));
				});
			}
			if (proto == QLatin1String("web"))
			{
				m_detectButton = new QPushButton(rjText(m_backend, "edit.web.detect"), w);
				m_detectButton->setObjectName(QStringLiteral("detectFields"));
				m_detectStatus = new QLabel(w);
				m_detectStatus->setWordWrap(true);
				m_detectStatus->setObjectName(QStringLiteral("detectStatus"));
				f->addRow(QString(), m_detectButton);
				f->addRow(QString(), m_detectStatus);
				connect(m_detectButton, &QPushButton::clicked, this, [this] {
					auto* urlEdit = qobject_cast<QLineEdit*>(m_opts.value(QStringLiteral("url")));
					auto* certCheck = qobject_cast<QCheckBox*>(m_opts.value(QStringLiteral("ignore_certificate")));
					const QJsonObject r = QJsonDocument::fromJson(m_backend->detectFields(m_id, urlEdit ? urlEdit->text().trimmed() : QString(), certCheck && certCheck->isChecked()).toUtf8()).object();
					if (r.contains(QStringLiteral("error")))
					{
						m_detectStatus->setText(rjText(m_backend, "edit.web.detect.error").arg(r.value(QStringLiteral("error")).toString()));
						return;
					}
					m_detectButton->setEnabled(false);
					m_detectStatus->setText(rjText(m_backend, "edit.web.detect.running"));
				});
			}
		}
	}
	// PowerShell-Transport und SNMP (vererbbar).
	m_psrp = new QComboBox(w);
	m_psrp->addItem(rjText(m_backend, "edit.psrp.inherit"), QStringLiteral("inherit"));
	m_psrp->addItem(rjText(m_backend, "edit.psrp.auto"), QStringLiteral("auto"));
	m_psrp->addItem(rjText(m_backend, "edit.psrp.winrm_https"), QStringLiteral("winrm_https"));
	m_psrp->addItem(rjText(m_backend, "edit.psrp.winrm_http"), QStringLiteral("winrm_http"));
	m_psrp->addItem(rjText(m_backend, "edit.psrp.ssh"), QStringLiteral("ssh"));
	m_psrpAuth = new QComboBox(w);
	// Negotiate (NTLM) zuerst: Das ist die Vorgabe und die einzige Wahl für lokale Konten ohne Domäne.
	m_psrpAuth->addItem(QStringLiteral("NEGOTIATE (NTLM)"), QStringLiteral("ntlm"));
	m_psrpAuth->addItem(QStringLiteral("KERBEROS"), QStringLiteral("kerberos"));
	m_psrpAuth->addItem(QStringLiteral("BASIC"), QStringLiteral("basic"));
	m_psrpPort = new QLineEdit(w);
	m_psrpVerify = new QCheckBox(rjText(m_backend, "edit.psrp.verify"), w);
	m_psrpVerify->setChecked(true);
	const QJsonValue pt = m_data.value(QStringLiteral("psrp_transport"));
	if (pt.isObject())
	{
		const QJsonObject t = pt.toObject();
		const QString type = t.value(QStringLiteral("type")).toString();
		if (type == QLatin1String("ssh") || type == QLatin1String("auto"))
			m_psrp->setCurrentIndex(m_psrp->findData(type));
		else
		{
			m_psrp->setCurrentIndex(m_psrp->findData(t.value(QStringLiteral("https")).toBool(true) ? QStringLiteral("winrm_https") : QStringLiteral("winrm_http")));
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
	{
		// PowerShell-Transport gilt für PowerShell- und Hyper-V-Ziele sowie als vererbbare Einstellung an Ordnern.
		const QString pn = target ? m_data.value(QStringLiteral("protocol")).toString() : QString();
		const bool showPs = !target || pn == QLatin1String("psrp") || pn == QLatin1String("hyperv");
		for (QWidget* x : { static_cast<QWidget*>(m_psrp), static_cast<QWidget*>(m_psrpAuth), static_cast<QWidget*>(m_psrpPort), static_cast<QWidget*>(m_psrpVerify) })
			f->setRowVisible(x, showPs);
	}
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
			else if (auto* cb = qobject_cast<QComboBox*>(i.value()))
			{
				const QString v = cb->currentData().toString();
				if (v == QLatin1String("auto"))
					o.remove(i.key());
				else
					o.insert(i.key(), v);
			}
		}
		if (m_akScript)
		{
			QJsonObject ak = o.value(QStringLiteral("auto_keys")).toObject();
			ak.insert(QStringLiteral("enabled"), m_akEnabled->isChecked());
			ak.insert(QStringLiteral("script"), m_akScript->toPlainText());
			ak.insert(QStringLiteral("trigger"), m_akTrigger->currentData().toString());
			ak.insert(QStringLiteral("wait_ms"), m_akWait->text().trimmed().toLongLong());
			ak.insert(QStringLiteral("max_ms"), m_akMax->text().trimmed().toLongLong());
			ak.insert(QStringLiteral("on_reconnect"), m_akReconnect->currentData().toString());
			// Nichts anlegen, solange die Tastenfolge aus und leer ist.
			if (!m_akEnabled->isChecked() && m_akScript->toPlainText().trimmed().isEmpty())
				o.remove(QStringLiteral("auto_keys"));
			else
				o.insert(QStringLiteral("auto_keys"), ak);
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
			if (p == QLatin1String("ssh") || p == QLatin1String("auto"))
				t.insert(QStringLiteral("type"), p);
			else
			{
				t.insert(QStringLiteral("type"), QStringLiteral("winrm"));
				t.insert(QStringLiteral("https"), p == QLatin1String("winrm_https"));
				t.insert(QStringLiteral("auth"), m_psrpAuth->currentData().toString());
				t.insert(QStringLiteral("verify_certificate"), m_psrpVerify->isChecked());
			}
			bool ok = false;
			const int port = m_psrpPort->text().toInt(&ok);
			if (ok && p != QLatin1String("auto"))
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
	if (m_akUser && m_akPass && m_dirty.contains(QStringLiteral("aux")))
	{
		QJsonObject a{ { QStringLiteral("name"), QStringLiteral("Sitzung") }, { QStringLiteral("username"), m_akUser->text() } };
		// Kennwort unverändert (kein Trimmen); leer heißt „unverändert lassen“.
		if (!m_akPass->text().isEmpty())
			a.insert(QStringLiteral("password"), m_akPass->text());
		cmd.insert(QStringLiteral("aux_credential"), a);
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

void PropertiesTab::saveAndClose()
{
	// Ohne Änderungen schließt die Schaltfläche einfach den Tab.
	if (!isDirty())
	{
		emit saveClosed();
		return;
	}
	m_closeAfter = true;
	save();
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
		m_closeAfter = false;
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
		m_closeAfter = false;
		m_status->setText(rjText(m_backend, "edit.error").arg(r.value(QStringLiteral("error")).toString()));
		emit indexChanged();
		return;
	}
	reload();
	emit indexChanged();
	const QString msg = r.value(QStringLiteral("notice")).toString(rjText(m_backend, "status.saved"));
	m_status->setText(msg);
	emit status(msg);
	if (m_closeAfter)
	{
		m_closeAfter = false;
		emit saveClosed();
	}
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

void PropertiesTab::wipeSecrets()
{
	if (m_password)
	{
		QString pw = m_password->text();
		pw.fill(QLatin1Char(' '));
		m_password->clear();
	}
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

// ------------------------------------------------------------------ SnmpTab

SnmpTab::SnmpTab(RjBackend* backend, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
{
	setProperty("editor", true);
	setObjectName(QStringLiteral("snmpTab"));
	auto* split = new QHBoxLayout(this);
	auto* left = new QVBoxLayout;
	m_list = new QListWidget(this);
	m_list->setAccessibleName(rjText(backend, "snmp.title"));
	auto* add = new QPushButton(rjText(backend, "snmp.new"), this);
	auto* del = new QPushButton(rjText(backend, "snmp.delete"), this);
	left->addWidget(m_list, 1);
	left->addWidget(add);
	left->addWidget(del);
	split->addLayout(left, 1);
	auto* f = new QFormLayout;
	auto line = [&](const char* label, bool secret) {
		auto* e = new QLineEdit(this);
		if (secret)
			e->setEchoMode(QLineEdit::Password);
		auto* l = new QLabel(rjText(backend, label), this);
		l->setBuddy(e);
		f->addRow(l, e);
		return e;
	};
	m_name = line("snmp.name", false);
	m_host = line("edit.host", false);
	m_port = line("edit.port", false);
	m_port->setPlaceholderText(rjText(backend, "edit.port.default").arg(161));
	m_version = new QComboBox(this);
	m_version->addItem(QStringLiteral("v2c"), QStringLiteral("v2c"));
	m_version->addItem(QStringLiteral("v3"), QStringLiteral("v3"));
	f->addRow(rjText(backend, "snmp.version"), m_version);
	m_community = line("snmp.community", true);
	m_user = line("snmp.user", false);
	m_auth = new QComboBox(this);
	for (const char* a : { "sha1", "sha256", "md5" })
		m_auth->addItem(QString::fromLatin1(a).toUpper(), QString::fromLatin1(a));
	f->addRow(rjText(backend, "snmp.auth"), m_auth);
	m_authPw = line("snmp.auth_password", true);
	m_priv = new QComboBox(this);
	m_priv->addItem(QStringLiteral("AES-128"), QStringLiteral("aes128"));
	m_priv->addItem(QStringLiteral("DES"), QStringLiteral("des"));
	m_priv->addItem(rjText(backend, "snmp.priv.none"), QStringLiteral("none"));
	f->addRow(rjText(backend, "snmp.priv"), m_priv);
	m_privPw = line("snmp.priv_password", true);
	m_message = new QLabel(this);
	m_message->setWordWrap(true);
	f->addRow(m_message);
	auto* saveB = new QPushButton(rjText(backend, "edit.save"), this);
	f->addRow(QString(), saveB);
	split->addLayout(f, 2);
	auto updateVersion = [this] {
		const bool v3 = m_version->currentData().toString() == QLatin1String("v3");
		m_community->setEnabled(!v3);
		for (QWidget* w : { static_cast<QWidget*>(m_user), static_cast<QWidget*>(m_auth), static_cast<QWidget*>(m_authPw), static_cast<QWidget*>(m_priv),
		                    static_cast<QWidget*>(m_privPw) })
			w->setEnabled(v3);
		m_privPw->setEnabled(v3 && m_priv->currentData().toString() != QLatin1String("none"));
	};
	connect(m_version, &QComboBox::currentIndexChanged, this, updateVersion);
	connect(m_priv, &QComboBox::currentIndexChanged, this, updateVersion);
	connect(m_list, &QListWidget::currentRowChanged, this, &SnmpTab::showDevice);
	connect(add, &QPushButton::clicked, this, [this] {
		m_list->setCurrentRow(-1);
		showDevice(-1);
		m_name->setFocus();
	});
	connect(del, &QPushButton::clicked, this, [this] {
		if (m_current.isEmpty())
			return;
		edit({ { QStringLiteral("op"), QStringLiteral("snmp_delete") }, { QStringLiteral("id"), m_current } });
		edit({ { QStringLiteral("op"), QStringLiteral("save") } });
		reload();
	});
	connect(saveB, &QPushButton::clicked, this, &SnmpTab::save);
	updateVersion();
	reload();
}

QString SnmpTab::edit(const QJsonObject& cmd)
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

void SnmpTab::reload()
{
	m_devices = QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("snmp_list") } }).toUtf8()).array();
	m_list->clear();
	for (const QJsonValue& d : m_devices)
		m_list->addItem(d.toObject().value(QStringLiteral("name")).toString() + QStringLiteral("  (") + d.toObject().value(QStringLiteral("version")).toString() + QLatin1Char(')'));
	int row = -1;
	for (int i = 0; i < m_devices.size(); ++i)
		if (m_devices[i].toObject().value(QStringLiteral("id")).toString() == m_current)
			row = i;
	m_list->setCurrentRow(row);
	showDevice(row);
}

void SnmpTab::showDevice(int row)
{
	const QJsonObject d = row >= 0 && row < m_devices.size() ? m_devices[row].toObject() : QJsonObject();
	m_current = d.value(QStringLiteral("id")).toString();
	m_name->setText(d.value(QStringLiteral("name")).toString());
	m_host->setText(d.value(QStringLiteral("host")).toString());
	m_port->setText(d.value(QStringLiteral("port")).isDouble() ? QString::number(d.value(QStringLiteral("port")).toInt()) : QString());
	m_version->setCurrentIndex(qMax(0, m_version->findData(d.value(QStringLiteral("version")).toString(QStringLiteral("v2c")))));
	m_user->setText(d.value(QStringLiteral("username")).toString());
	m_auth->setCurrentIndex(qMax(0, m_auth->findData(d.value(QStringLiteral("auth_proto")).toString())));
	m_priv->setCurrentIndex(qMax(0, m_priv->findData(d.value(QStringLiteral("priv_proto")).toString())));
	// Kennwörter werden nie angezeigt; leer lassen heißt „unverändert“.
	const QString keep = m_current.isEmpty() ? QString() : rjText(m_backend, "edit.cred.password.keep");
	for (QLineEdit* e : { m_community, m_authPw, m_privPw })
	{
		e->clear();
		e->setPlaceholderText(keep);
	}
	m_message->clear();
}

void SnmpTab::save()
{
	QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("snmp_save") },
		             { QStringLiteral("name"), m_name->text() },
		             { QStringLiteral("host"), m_host->text() },
		             { QStringLiteral("version"), m_version->currentData().toString() },
		             { QStringLiteral("username"), m_user->text() },
		             { QStringLiteral("auth_proto"), m_auth->currentData().toString() },
		             { QStringLiteral("priv_proto"), m_priv->currentData().toString() } };
	if (!m_current.isEmpty())
		cmd.insert(QStringLiteral("id"), m_current);
	if (!m_port->text().isEmpty())
		cmd.insert(QStringLiteral("port"), m_port->text());
	// Kennwörter unverändert übergeben (kein Trimmen, keine Normalisierung).
	const char* keys[] = { "community", "auth_password", "priv_password" };
	QLineEdit* fields[] = { m_community, m_authPw, m_privPw };
	for (int i = 0; i < 3; ++i)
		if (!fields[i]->text().isEmpty())
			cmd.insert(QString::fromLatin1(keys[i]), fields[i]->text());
	QString json = QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact));
	const QJsonObject r = parse(m_backend->edit(json));
	json.fill(QLatin1Char(' '));
	wipeSecrets();
	if (r.contains(QStringLiteral("error")))
	{
		m_message->setText(rjText(m_backend, "edit.error").arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	m_current = r.value(QStringLiteral("id")).toString();
	reload();
	m_message->setText(rjText(m_backend, "status.saved"));
	emit status(rjText(m_backend, "status.saved"));
}

void SnmpTab::wipeSecrets()
{
	for (QLineEdit* e : { m_community, m_authPw, m_privPw })
	{
		QString t = e->text();
		t.fill(QLatin1Char(' '));
		e->clear();
	}
}

// ------------------------------------------------------------ MacroTab

namespace {
QJsonObject macroCall(RjBackend* b, const QJsonObject& o)
{
	return QJsonDocument::fromJson(b->edit(QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Compact))).toUtf8()).object();
}
}

MacroTab::MacroTab(RjBackend* backend, const QString& id, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
  , m_id(id)
{
	setObjectName(QStringLiteral("macroTab"));
	auto* v = new QVBoxLayout(this);
	auto* top = new QFormLayout;
	m_name = new QLineEdit(this);
	top->addRow(new QLabel(rjText(backend, "macro.name"), this), m_name);
	v->addLayout(top);
	auto* hint = new QLabel(rjText(backend, "macro.hint"), this);
	hint->setWordWrap(true);
	v->addWidget(hint);
	v->addWidget(new QLabel(rjText(backend, "macro.steps"), this));

	auto* mid = new QHBoxLayout;
	m_table = new QTableWidget(0, 3, this);
	m_table->setHorizontalHeaderLabels({ rjText(backend, "macro.col.nr"), rjText(backend, "macro.col.kind"), rjText(backend, "macro.col.content") });
	m_table->horizontalHeader()->setStretchLastSection(true);
	m_table->verticalHeader()->hide();
	m_table->setSelectionBehavior(QAbstractItemView::SelectRows);
	m_table->setSelectionMode(QAbstractItemView::SingleSelection);
	m_table->setEditTriggers(QAbstractItemView::NoEditTriggers);
	m_table->setMinimumHeight(150);
	mid->addWidget(m_table, 1);
	auto* side = new QVBoxLayout;
	auto* add = new QPushButton(rjText(backend, "macro.add"), this);
	auto* addMenu = new QMenu(add);
	addMenu->addAction(rjText(backend, "macro.add.send"), this, [this] { addStep(QStringLiteral("send")); });
	addMenu->addAction(rjText(backend, "macro.add.wait"), this, [this] { addStep(QStringLiteral("wait")); });
	add->setMenu(addMenu);
	auto* up = new QPushButton(rjText(backend, "macro.up"), this);
	auto* down = new QPushButton(rjText(backend, "macro.down"), this);
	auto* rem = new QPushButton(rjText(backend, "macro.remove"), this);
	side->addWidget(add);
	side->addWidget(up);
	side->addWidget(down);
	side->addWidget(rem);
	side->addStretch(1);
	mid->addLayout(side);
	v->addLayout(mid);

	// Formulare je Schritt
	m_stack = new QStackedWidget(this);
	m_stack->addWidget(new QWidget(this)); // 0: nichts gewählt
	auto* sendPage = new QWidget(this);
	{
		auto* f = new QFormLayout(sendPage);
		m_sendText = new QLineEdit(sendPage);
		m_sendEnter = new QCheckBox(rjText(backend, "macro.send.enter"), sendPage);
		m_sendInsert = new QComboBox(sendPage);
		m_sendInsert->addItem(rjText(backend, "macro.send.insert"), QString());
		m_sendInsert->addItem(rjText(backend, "macro.send.ins.enter"), QStringLiteral("{key:enter}"));
		m_sendInsert->addItem(rjText(backend, "macro.send.ins.tab"), QStringLiteral("{key:tab}"));
		m_sendInsert->addItem(rjText(backend, "macro.send.ins.esc"), QStringLiteral("{key:esc}"));
		m_sendInsert->addItem(rjText(backend, "macro.send.ins.ctrlc"), QStringLiteral("{key:ctrl-c}"));
		m_sendInsert->addItem(rjText(backend, "macro.send.ins.pw"), QStringLiteral("{cred:password}"));
		m_sendInsert->addItem(rjText(backend, "macro.send.ins.user"), QStringLiteral("{cred:username}"));
		f->addRow(new QLabel(rjText(backend, "macro.send.text"), sendPage), m_sendText);
		f->addRow(QString(), m_sendEnter);
		f->addRow(QString(), m_sendInsert);
	}
	m_stack->addWidget(sendPage); // 1
	auto* waitPage = new QWidget(this);
	{
		auto* f = new QFormLayout(waitPage);
		m_waitKind = new QComboBox(waitPage);
		m_waitKind->addItem(rjText(backend, "macro.wait.text"), QStringLiteral("text"));
		m_waitKind->addItem(rjText(backend, "macro.wait.regex"), QStringLiteral("regex"));
		m_waitPattern = new QLineEdit(waitPage);
		m_waitTimeout = new QSpinBox(waitPage);
		m_waitTimeout->setRange(1, 600);
		m_waitTimeout->setSuffix(QStringLiteral(" s"));
		m_waitErrors = new QPlainTextEdit(waitPage);
		m_waitErrors->setMaximumHeight(70);
		f->addRow(new QLabel(rjText(backend, "macro.wait.kind"), waitPage), m_waitKind);
		f->addRow(new QLabel(rjText(backend, "macro.wait.pattern"), waitPage), m_waitPattern);
		f->addRow(new QLabel(rjText(backend, "macro.wait.timeout"), waitPage), m_waitTimeout);
		f->addRow(new QLabel(rjText(backend, "macro.wait.errors"), waitPage), m_waitErrors);
	}
	m_stack->addWidget(waitPage); // 2
	auto* advPage = new QWidget(this);
	{
		auto* l = new QVBoxLayout(advPage);
		auto* t = new QLabel(rjText(backend, "macro.adv.hint"), advPage);
		t->setWordWrap(true);
		m_json = new QPlainTextEdit(advPage);
		m_json->setMaximumHeight(140);
		m_jsonError = new QLabel(advPage);
		m_jsonError->setObjectName(QStringLiteral("fieldError"));
		l->addWidget(t);
		l->addWidget(m_json);
		l->addWidget(m_jsonError);
	}
	m_stack->addWidget(advPage); // 3
	v->addWidget(m_stack);

	m_linked = new QLabel(this);
	m_linked->setWordWrap(true);
	v->addWidget(m_linked);
	m_status = new QLabel(this);
	m_status->setWordWrap(true);
	m_status->setObjectName(QStringLiteral("fieldError"));
	v->addWidget(m_status);
	m_dirtyLabel = new QLabel(rjText(backend, "macro.dirty"), this);
	m_dirtyLabel->setObjectName(QStringLiteral("dirtyLabel"));
	m_dirtyLabel->hide();
	auto* buttons = new QHBoxLayout;
	auto* saveB = new QPushButton(rjText(backend, "macro.save"), this);
	auto* discardB = new QPushButton(rjText(backend, "macro.discard"), this);
	saveB->setDefault(true);
	buttons->addWidget(m_dirtyLabel);
	buttons->addStretch(1);
	buttons->addWidget(discardB);
	buttons->addWidget(saveB);
	v->addLayout(buttons);

	connect(m_name, &QLineEdit::textEdited, this, [this] {
		if (!m_loading)
		{
			markDirty();
			emit titleChanged();
		}
	});
	connect(m_table, &QTableWidget::currentCellChanged, this, [this](int row, int, int, int) { showStep(row); });
	connect(up, &QPushButton::clicked, this, [this] { moveStep(-1); });
	connect(down, &QPushButton::clicked, this, [this] { moveStep(1); });
	connect(rem, &QPushButton::clicked, this, &MacroTab::removeStep);
	connect(m_sendText, &QLineEdit::textEdited, this, &MacroTab::storeStep);
	connect(m_sendEnter, &QCheckBox::toggled, this, &MacroTab::storeStep);
	connect(m_sendInsert, &QComboBox::activated, this, [this](int i) {
		const QString ins = m_sendInsert->itemData(i).toString();
		if (!ins.isEmpty())
		{
			m_sendText->insert(ins);
			storeStep();
		}
		m_sendInsert->setCurrentIndex(0);
	});
	connect(m_waitKind, &QComboBox::activated, this, [this](int) { storeStep(); });
	connect(m_waitPattern, &QLineEdit::textEdited, this, &MacroTab::storeStep);
	connect(m_waitTimeout, &QSpinBox::valueChanged, this, [this](int) { storeStep(); });
	connect(m_waitErrors, &QPlainTextEdit::textChanged, this, [this] { storeStep(); });
	connect(m_json, &QPlainTextEdit::textChanged, this, [this] { storeStep(); });
	connect(saveB, &QPushButton::clicked, this, [this] {
		if (save())
			emit saved();
	});
	connect(discardB, &QPushButton::clicked, this, [this] {
		m_dirty = false;
		emit discarded();
	});
	load();
}

QString MacroTab::title() const
{
	const QString n = m_name->text().trimmed();
	return n.isEmpty() ? (m_origName.isEmpty() ? rjText(m_backend, "macro.title.default") : m_origName) : n;
}

void MacroTab::markDirty()
{
	m_dirty = true;
	m_dirtyLabel->show();
	m_status->clear();
}

void MacroTab::load()
{
	m_loading = true;
	const QJsonObject r = macroCall(m_backend, { { QStringLiteral("op"), QStringLiteral("macro_get") }, { QStringLiteral("id"), m_id } });
	m_origName = r.value(QStringLiteral("name")).toString();
	m_name->setText(m_origName);
	m_steps = r.value(QStringLiteral("steps")).toArray();
	QStringList names;
	for (const QJsonValue& l : r.value(QStringLiteral("linked")).toArray())
		names << l.toObject().value(QStringLiteral("name")).toString();
	m_linked->setText(names.isEmpty() ? rjText(m_backend, "macro.unlinked") : rjText(m_backend, "macro.linked").arg(names.join(QStringLiteral(", "))));
	m_dirty = false;
	m_dirtyLabel->hide();
	m_loading = false;
	refreshTable(m_steps.isEmpty() ? -1 : 0);
}

QString MacroTab::summary(const QJsonObject& step) const
{
	const QString kind = step.value(QStringLiteral("step")).toString();
	if (kind == QLatin1String("send"))
	{
		QString t = step.value(QStringLiteral("text")).toString();
		t.replace(QStringLiteral("{key:enter}"), QStringLiteral(" ⏎"));
		t.replace(QStringLiteral("{cred:password}"), QStringLiteral("‹Kennwort›"));
		t.replace(QStringLiteral("{cred:username}"), QStringLiteral("‹Benutzer›"));
		return t;
	}
	if (kind == QLatin1String("wait"))
	{
		const QJsonObject pat = step.value(QStringLiteral("pattern")).toObject();
		const bool rx = pat.contains(QStringLiteral("regex"));
		const QString text = rx ? pat.value(QStringLiteral("regex")).toString() : pat.value(QStringLiteral("text")).toString();
		return QStringLiteral("%1%2%3 (%4 s)").arg(rx ? QStringLiteral("/") : QStringLiteral("„"), text, rx ? QStringLiteral("/") : QStringLiteral("“")).arg(step.value(QStringLiteral("timeout_ms")).toInt(15000) / 1000);
	}
	if (kind == QLatin1String("expect_any"))
		return QStringLiteral("%1 Zweige").arg(step.value(QStringLiteral("branches")).toArray().size());
	return QString::fromUtf8(QJsonDocument(step).toJson(QJsonDocument::Compact));
}

void MacroTab::refreshTable(int select)
{
	const bool was = m_loading;
	m_loading = true;
	m_table->setRowCount(0);
	for (int i = 0; i < m_steps.size(); ++i)
	{
		const QJsonObject st = m_steps[i].toObject();
		const QString kind = st.value(QStringLiteral("step")).toString();
		const QString kindText = kind == QLatin1String("send")         ? rjText(m_backend, "macro.kind.send")
		                         : kind == QLatin1String("wait")       ? rjText(m_backend, "macro.kind.wait")
		                         : kind == QLatin1String("expect_any") ? rjText(m_backend, "macro.kind.any")
		                                                               : rjText(m_backend, "macro.kind.other");
		m_table->insertRow(i);
		m_table->setItem(i, 0, new QTableWidgetItem(QString::number(i + 1)));
		m_table->setItem(i, 1, new QTableWidgetItem(kindText));
		m_table->setItem(i, 2, new QTableWidgetItem(summary(st)));
	}
	m_table->resizeColumnToContents(0);
	m_table->resizeColumnToContents(1);
	m_loading = was;
	if (select >= 0 && select < m_steps.size())
	{
		m_table->setCurrentCell(select, 2);
		showStep(select);
	}
	else
		showStep(-1);
}

void MacroTab::showStep(int row)
{
	if (m_loading)
		return;
	m_loading = true;
	if (row < 0 || row >= m_steps.size())
		m_stack->setCurrentIndex(0);
	else
	{
		const QJsonObject st = m_steps[row].toObject();
		const QString kind = st.value(QStringLiteral("step")).toString();
		if (kind == QLatin1String("send"))
		{
			QString t = st.value(QStringLiteral("text")).toString();
			const QString enter = QStringLiteral("{key:enter}");
			const bool e = t.endsWith(enter);
			if (e)
				t.chop(enter.size());
			m_sendText->setText(t);
			m_sendEnter->setChecked(e);
			m_stack->setCurrentIndex(1);
		}
		else if (kind == QLatin1String("wait"))
		{
			const QJsonObject pat = st.value(QStringLiteral("pattern")).toObject();
			const bool rx = pat.contains(QStringLiteral("regex"));
			m_waitKind->setCurrentIndex(rx ? 1 : 0);
			m_waitPattern->setText(rx ? pat.value(QStringLiteral("regex")).toString() : pat.value(QStringLiteral("text")).toString());
			m_waitTimeout->setValue(qBound(1, st.value(QStringLiteral("timeout_ms")).toInt(15000) / 1000, 600));
			QStringList errs;
			for (const QJsonValue& e : st.value(QStringLiteral("errors")).toArray())
			{
				const QJsonObject eo = e.toObject();
				errs << (eo.contains(QStringLiteral("regex")) ? eo.value(QStringLiteral("regex")).toString() : eo.value(QStringLiteral("text")).toString());
			}
			m_waitErrors->setPlainText(errs.join(QLatin1Char('\n')));
			m_stack->setCurrentIndex(2);
		}
		else
		{
			m_json->setPlainText(QString::fromUtf8(QJsonDocument(st).toJson(QJsonDocument::Indented)));
			m_jsonError->clear();
			m_stack->setCurrentIndex(3);
		}
	}
	m_loading = false;
}

void MacroTab::storeStep()
{
	if (m_loading)
		return;
	const int row = m_table->currentRow();
	if (row < 0 || row >= m_steps.size())
		return;
	QJsonObject st = m_steps[row].toObject();
	const QString kind = st.value(QStringLiteral("step")).toString();
	if (kind == QLatin1String("send"))
		st.insert(QStringLiteral("text"), m_sendText->text() + (m_sendEnter->isChecked() ? QStringLiteral("{key:enter}") : QString()));
	else if (kind == QLatin1String("wait"))
	{
		const bool rx = m_waitKind->currentData().toString() == QLatin1String("regex");
		st.insert(QStringLiteral("pattern"), QJsonObject{ { rx ? QStringLiteral("regex") : QStringLiteral("text"), m_waitPattern->text() } });
		st.insert(QStringLiteral("timeout_ms"), m_waitTimeout->value() * 1000);
		QJsonArray errs;
		for (const QString& line : m_waitErrors->toPlainText().split(QLatin1Char('\n'), Qt::SkipEmptyParts))
			errs.append(QJsonObject{ { QStringLiteral("text"), line } });
		st.insert(QStringLiteral("errors"), errs);
	}
	else
	{
		QJsonParseError err;
		const QJsonDocument d = QJsonDocument::fromJson(m_json->toPlainText().toUtf8(), &err);
		if (err.error != QJsonParseError::NoError || !d.isObject())
		{
			m_jsonError->setText(rjText(m_backend, "macro.adv.bad").arg(err.errorString()));
			return;
		}
		m_jsonError->clear();
		st = d.object();
	}
	m_steps[row] = st;
	if (QTableWidgetItem* it = m_table->item(row, 2))
		it->setText(summary(st));
	markDirty();
}

void MacroTab::addStep(const QString& kind)
{
	QJsonObject st;
	if (kind == QLatin1String("send"))
		st = QJsonObject{ { QStringLiteral("step"), QStringLiteral("send") }, { QStringLiteral("text"), QString() } };
	else
		st = QJsonObject{ { QStringLiteral("step"), QStringLiteral("wait") },
			              { QStringLiteral("pattern"), QJsonObject{ { QStringLiteral("text"), QString() } } },
			              { QStringLiteral("errors"), QJsonArray() },
			              { QStringLiteral("timeout_ms"), 15000 } };
	const int at = m_table->currentRow() >= 0 ? m_table->currentRow() + 1 : m_steps.size();
	m_steps.insert(at, st);
	markDirty();
	refreshTable(at);
	(kind == QLatin1String("send") ? static_cast<QWidget*>(m_sendText) : static_cast<QWidget*>(m_waitPattern))->setFocus();
}

void MacroTab::moveStep(int delta)
{
	const int row = m_table->currentRow();
	const int to = row + delta;
	if (row < 0 || to < 0 || to >= m_steps.size())
		return;
	const QJsonValue v = m_steps[row];
	m_steps.removeAt(row);
	m_steps.insert(to, v);
	markDirty();
	refreshTable(to);
}

void MacroTab::removeStep()
{
	const int row = m_table->currentRow();
	if (row < 0 || row >= m_steps.size())
		return;
	m_steps.removeAt(row);
	markDirty();
	refreshTable(m_steps.isEmpty() ? -1 : qMin(row, int(m_steps.size()) - 1));
}

bool MacroTab::save()
{
	const QString name = m_name->text().trimmed();
	if (name.isEmpty())
	{
		m_status->setText(rjText(m_backend, "macro.err.name"));
		return false;
	}
	// Muster prüfen: leere Muster und ungültige reguläre Ausdrücke abfangen.
	for (int i = 0; i < m_steps.size(); ++i)
	{
		const QJsonObject st = m_steps[i].toObject();
		if (st.value(QStringLiteral("step")).toString() != QLatin1String("wait"))
			continue;
		const QJsonObject pat = st.value(QStringLiteral("pattern")).toObject();
		const bool rx = pat.contains(QStringLiteral("regex"));
		const QString text = rx ? pat.value(QStringLiteral("regex")).toString() : pat.value(QStringLiteral("text")).toString();
		if (text.isEmpty() || (rx && !QRegularExpression(text).isValid()))
		{
			m_table->setCurrentCell(i, 2);
			m_status->setText(QStringLiteral("%1 %2: %3").arg(rjText(m_backend, "macro.col.nr"), QString::number(i + 1), text.isEmpty() ? rjText(m_backend, "macro.wait.pattern") : QRegularExpression(text).errorString()));
			return false;
		}
	}
	const QJsonObject r = macroCall(m_backend, { { QStringLiteral("op"), QStringLiteral("update_macro") }, { QStringLiteral("id"), m_id }, { QStringLiteral("name"), name }, { QStringLiteral("steps"), m_steps } });
	if (r.contains(QStringLiteral("error")))
	{
		m_status->setText(r.value(QStringLiteral("error")).toString());
		return false;
	}
	m_origName = name;
	m_dirty = false;
	m_dirtyLabel->hide();
	emit indexChanged();
	return true;
}

// ------------------------------------------------------------ TerminalSettingsTab

namespace {

/// Vorschau: Beispieltext, Cursor, Auswahl und die 16 ANSI-Farben im gewählten Schema.
class TerminalPreview : public QWidget
{
public:
	explicit TerminalPreview(QWidget* parent)
	  : QWidget(parent)
	{
		setMinimumHeight(150);
		setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
	}
	void set(const QFont& font, const QColor& fg, const QColor& bg, const QColor& cursor, const QColor& selection, const QStringList& ansi)
	{
		m_font = font;
		m_fg = fg;
		m_bg = bg;
		m_cursor = cursor;
		m_sel = selection;
		m_ansi = ansi;
		update();
	}

protected:
	void paintEvent(QPaintEvent*) override
	{
		QPainter p(this);
		p.fillRect(rect(), m_bg);
		p.setFont(m_font);
		const QFontMetrics fm(m_font);
		const int ch = fm.height(), cw = qMax(1, fm.horizontalAdvance(QLatin1Char('M')));
		int y = 8 + fm.ascent();
		auto text = [&](int x, const QString& t, const QColor& c) {
			p.setPen(c);
			p.drawText(x, y, t);
			return x + fm.horizontalAdvance(t);
		};
		auto col = [&](int i, const QColor& dflt) { return i < m_ansi.size() ? QColor(m_ansi.at(i)) : dflt; };
		int x = 10;
		x = text(x, QStringLiteral("benutzer@rechner"), col(2, m_fg));
		x = text(x, QStringLiteral(":"), m_fg);
		x = text(x, QStringLiteral("~"), col(4, m_fg));
		x = text(x, QStringLiteral("$ ls --color=auto"), m_fg);
		y += ch;
		x = 10;
		x = text(x, QStringLiteral("Dokumente  "), col(4, m_fg));
		x = text(x, QStringLiteral("skript.sh  "), col(2, m_fg));
		x = text(x, QStringLiteral("archiv.tar  "), col(1, m_fg));
		text(x, QStringLiteral("notiz.txt"), m_fg);
		y += ch;
		x = 10;
		x = text(x, QStringLiteral("Fehler: "), col(1, m_fg));
		x = text(x, QStringLiteral("Verbindung "), col(3, m_fg));
		// Ausgewählter Text
		const QString selText = QStringLiteral("ausgewählt");
		p.fillRect(QRect(x, y - fm.ascent(), fm.horizontalAdvance(selText), ch), m_sel);
		x = text(x, selText, m_fg);
		x += cw;
		QColor cur = m_cursor;
		cur.setAlpha(160);
		p.fillRect(QRect(x, y - fm.ascent(), cw, ch), cur);
		// Farbstreifen
		y += ch;
		const int sw = qMax(14, cw * 2), top = y - fm.ascent() + 4;
		for (int i = 0; i < 16 && i < m_ansi.size(); ++i)
		{
			p.fillRect(QRect(10 + (i % 8) * (sw + 3), top + (i / 8) * (ch / 2 + 2), sw, ch / 2), QColor(m_ansi.at(i)));
		}
	}

private:
	QFont m_font;
	QColor m_fg, m_bg, m_cursor, m_sel;
	QStringList m_ansi;
};

}

TerminalSettingsTab::TerminalSettingsTab(RjBackend* backend, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
{
	setObjectName(QStringLiteral("terminalSettings"));
	m_schemes = QJsonDocument::fromJson(m_backend->terminalSchemes().toUtf8()).array();
	auto* v = new QVBoxLayout(this);
	auto* hint = new QLabel(rjText(backend, "term.hint"), this);
	hint->setWordWrap(true);
	v->addWidget(hint);
	auto* f = new QFormLayout;
	m_system = new QCheckBox(rjText(backend, "term.font.system"), this);
	m_font = new QFontComboBox(this);
	m_font->setFontFilters(QFontComboBox::MonospacedFonts);
	m_size = new QSpinBox(this);
	m_size->setRange(6, 40);
	m_size->setSuffix(QStringLiteral(" pt"));
	m_scrollback = new QSpinBox(this);
	m_scrollback->setRange(100, 200000);
	m_scrollback->setSingleStep(1000);
	m_scrollback->setSuffix(rjText(backend, "term.scrollback.suffix"));
	m_scheme = new QComboBox(this);
	for (const QJsonValue& sv : std::as_const(m_schemes))
		m_scheme->addItem(sv.toObject().value(QStringLiteral("label")).toString(), sv.toObject().value(QStringLiteral("id")).toString());
	m_custom = new QLabel(rjText(backend, "term.custom"), this);
	m_custom->setObjectName(QStringLiteral("dirtyLabel"));
	m_custom->hide();
	f->addRow(QString(), m_system);
	f->addRow(new QLabel(rjText(backend, "term.font"), this), m_font);
	f->addRow(new QLabel(rjText(backend, "term.size"), this), m_size);
	f->addRow(new QLabel(rjText(backend, "term.scrollback"), this), m_scrollback);
	m_copyOnSelect = new QCheckBox(rjText(backend, "term.copy_on_select"), this);
	m_confirmPaste = new QCheckBox(rjText(backend, "term.confirm_paste"), this);
	f->addRow(QString(), m_copyOnSelect);
	f->addRow(QString(), m_confirmPaste);
	f->addRow(new QLabel(rjText(backend, "term.scheme"), this), m_scheme);
	f->addRow(QString(), m_custom);
	const char* keys[4] = { "fg", "bg", "cursor", "selection" };
	const char* labels[4] = { "term.fg", "term.bg", "term.cursor", "term.selection" };
	for (int i = 0; i < 4; ++i)
	{
		m_hex[i] = new QLineEdit(this);
		m_hex[i]->setMaxLength(7);
		m_hex[i]->setPlaceholderText(QStringLiteral("#rrggbb"));
		m_hex[i]->setMaximumWidth(110);
		m_swatch[i] = new QLabel(this);
		m_swatch[i]->setFixedSize(40, 20);
		auto* rowW = new QWidget(this);
		auto* rl = new QHBoxLayout(rowW);
		rl->setContentsMargins(0, 0, 0, 0);
		rl->addWidget(m_hex[i]);
		rl->addWidget(m_swatch[i]);
		rl->addStretch(1);
		f->addRow(new QLabel(rjText(backend, labels[i]), this), rowW);
		const QString key = QString::fromLatin1(keys[i]);
		connect(m_hex[i], &QLineEdit::editingFinished, this, [this, i, key] {
			if (!m_updating)
				send({ { key, m_hex[i]->text().trimmed() } });
		});
	}
	v->addLayout(f);
	m_preview = new TerminalPreview(this);
	v->addWidget(m_preview);
	m_message = new QLabel(this);
	m_message->setWordWrap(true);
	m_message->setObjectName(QStringLiteral("fieldError"));
	v->addWidget(m_message);
	auto* reset = new QPushButton(rjText(backend, "term.reset"), this);
	auto* row = new QHBoxLayout;
	row->addWidget(reset);
	row->addStretch(1);
	v->addLayout(row);
	v->addStretch(1);

	connect(m_system, &QCheckBox::toggled, this, [this](bool on) {
		if (m_updating)
			return;
		m_font->setEnabled(!on);
		send({ { QStringLiteral("font_family"), on ? QString() : m_font->currentFont().family() } });
	});
	connect(m_font, &QFontComboBox::currentFontChanged, this, [this](const QFont& font) {
		if (!m_updating && !m_system->isChecked())
			send({ { QStringLiteral("font_family"), font.family() } });
	});
	connect(m_size, &QSpinBox::valueChanged, this, [this](int value) {
		if (!m_updating)
			send({ { QStringLiteral("font_size"), value } });
	});
	connect(m_copyOnSelect, &QCheckBox::toggled, this, [this](bool on) {
		if (!m_updating)
			send({ { QStringLiteral("copy_on_select"), on } });
	});
	connect(m_confirmPaste, &QCheckBox::toggled, this, [this](bool on) {
		if (!m_updating)
			send({ { QStringLiteral("confirm_paste"), on } });
	});
	connect(m_scrollback, &QSpinBox::editingFinished, this, [this] {
		if (!m_updating)
			send({ { QStringLiteral("scrollback"), m_scrollback->value() } });
	});
	connect(m_scheme, &QComboBox::activated, this, [this](int i) {
		if (!m_updating)
			send({ { QStringLiteral("scheme"), m_scheme->itemData(i).toString() } });
	});
	connect(reset, &QPushButton::clicked, this, [this] {
		send({ { QStringLiteral("scheme"), QStringLiteral("standard-dunkel") },
		       { QStringLiteral("font_family"), QString() },
		       { QStringLiteral("font_size"), 11 },
		       { QStringLiteral("scrollback"), 10000 },
		       { QStringLiteral("copy_on_select"), true },
		       { QStringLiteral("confirm_paste"), true },
		       { QStringLiteral("fg"), QString() },
		       { QStringLiteral("bg"), QString() },
		       { QStringLiteral("cursor"), QString() },
		       { QStringLiteral("selection"), QString() } });
	});
	show(QJsonDocument::fromJson(m_backend->terminalSettings().toUtf8()).object());
}

void TerminalSettingsTab::send(const QJsonObject& change)
{
	const QJsonObject r = QJsonDocument::fromJson(m_backend->setTerminalSettings(QString::fromUtf8(QJsonDocument(change).toJson(QJsonDocument::Compact))).toUtf8()).object();
	if (r.contains(QStringLiteral("error")))
	{
		// Eingabe abgelehnt: Fehler zeigen, Anzeige auf den gültigen Stand zurücksetzen.
		const QString err = r.value(QStringLiteral("error")).toString();
		show(QJsonDocument::fromJson(m_backend->terminalSettings().toUtf8()).object());
		m_message->setText(err);
		return;
	}
	show(r);
}

void TerminalSettingsTab::show(const QJsonObject& s)
{
	m_updating = true;
	m_message->clear();
	const QString family = s.value(QStringLiteral("font_family")).toString();
	m_system->setChecked(family.isEmpty());
	m_font->setEnabled(!family.isEmpty());
	m_font->setCurrentFont(family.isEmpty() ? QFontDatabase::systemFont(QFontDatabase::FixedFont) : QFont(family));
	m_size->setValue(qRound(s.value(QStringLiteral("font_size")).toDouble(11.0)));
	m_scrollback->setValue(s.value(QStringLiteral("scrollback")).toInt(10000));
	m_copyOnSelect->setChecked(s.value(QStringLiteral("copy_on_select")).toBool(true));
	m_confirmPaste->setChecked(s.value(QStringLiteral("confirm_paste")).toBool(true));
	m_scheme->setCurrentIndex(qMax(0, m_scheme->findData(s.value(QStringLiteral("scheme")).toString())));
	m_custom->setVisible(s.value(QStringLiteral("custom")).toBool());
	const char* keys[4] = { "fg", "bg", "cursor", "selection" };
	for (int i = 0; i < 4; ++i)
	{
		const QString c = s.value(QString::fromLatin1(keys[i])).toString();
		m_hex[i]->setText(c);
		m_swatch[i]->setStyleSheet(QStringLiteral("background-color: %1; border: 1px solid gray;").arg(c));
	}
	QStringList ansi;
	for (const QJsonValue& sv : std::as_const(m_schemes))
		if (sv.toObject().value(QStringLiteral("id")).toString() == s.value(QStringLiteral("scheme")).toString())
			for (const QJsonValue& a : sv.toObject().value(QStringLiteral("ansi")).toArray())
				ansi << a.toString();
	QFont pf = family.isEmpty() ? QFontDatabase::systemFont(QFontDatabase::FixedFont) : QFont(family);
	pf.setStyleHint(QFont::Monospace);
	pf.setPointSizeF(s.value(QStringLiteral("font_size")).toDouble(11.0));
	static_cast<TerminalPreview*>(m_preview)->set(pf, QColor(s.value(QStringLiteral("fg")).toString()), QColor(s.value(QStringLiteral("bg")).toString()),
	                                              QColor(s.value(QStringLiteral("cursor")).toString()), QColor(s.value(QStringLiteral("selection")).toString()), ansi);
	m_updating = false;
	TerminalView::applyToAll(s);
}

// ------------------------------------------------------------ UnlockOverlay

UnlockOverlay::UnlockOverlay(RjBackend* backend, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
{
	setObjectName(QStringLiteral("unlockOverlay"));
	setAttribute(Qt::WA_StyledBackground, true);
	setAutoFillBackground(false);
	// Großes Logo unten rechts (außerhalb des Layouts, unter der Eingabebox).
	m_logo = new QLabel(this);
	m_logo->setObjectName(QStringLiteral("unlockLogo"));
	m_logo->setAttribute(Qt::WA_TransparentForMouseEvents, true);
	m_logo->setAccessibleName(QStringLiteral("remotejoe"));
	auto* outer = new QVBoxLayout(this);
	auto* box = new QFrame(this);
	box->setObjectName(QStringLiteral("unlockBox"));
	box->setFrameShape(QFrame::StyledPanel);
	box->setMaximumWidth(620);
	box->setAutoFillBackground(true);
	outer->addStretch(1);
	auto* row = new QHBoxLayout;
	row->addStretch(1);
	row->addWidget(box, 3);
	row->addStretch(1);
	outer->addLayout(row);
	outer->addStretch(1);
	auto* v = new QVBoxLayout(box);
	m_title = new QLabel(rjText(backend, "unlock.title"), box);
	QFont tf = m_title->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.4);
	tf.setBold(true);
	m_title->setFont(tf);
	v->addWidget(m_title);
	m_hint = new QLabel(box);
	m_hint->setWordWrap(true);
	m_hint->hide();
	v->addWidget(m_hint);
	m_form = new QFormLayout;
	auto* fs = new QFileSystemModel(this);
	fs->setRootPath(QString());
	m_path = new QLineEdit(box);
	m_path->setCompleter(new QCompleter(fs, this));
	m_browse = new QPushButton(rjText(backend, "unlock.browse"), box);
	auto* pathRow = new QWidget(box);
	auto* pl = new QHBoxLayout(pathRow);
	pl->setContentsMargins(0, 0, 0, 0);
	pl->addWidget(m_path, 1);
	pl->addWidget(m_browse);
	m_password = new QLineEdit(box);
	m_password->setEchoMode(QLineEdit::Password);
	m_password2 = new QLineEdit(box);
	m_password2->setEchoMode(QLineEdit::Password);
	m_useKey = new QCheckBox(rjText(backend, "unlock.usekey"), box);
	m_useKey->setToolTip(rjText(backend, "unlock.usekey.tip"));
	m_keyFile = new QLineEdit(box);
	m_keyFile->setCompleter(new QCompleter(fs, this));
	m_browseKey = new QPushButton(rjText(backend, "unlock.browse.key.button"), box);
	m_keyRow = new QWidget(box);
	auto* kl = new QHBoxLayout(m_keyRow);
	kl->setContentsMargins(0, 0, 0, 0);
	kl->addWidget(m_keyFile, 1);
	kl->addWidget(m_browseKey);
	m_makeKey = new QCheckBox(rjText(backend, "unlock.makekey"), box);
	m_remember = new QCheckBox(rjText(backend, "unlock.remember"), box);
	m_remember->setToolTip(rjText(backend, "unlock.remember.tip"));
	m_strength = new QLabel(box);
	m_strength->setWordWrap(true);
	auto addRow = [&](const char* label, QWidget* field, QWidget* buddy) {
		auto* l = new QLabel(rjText(backend, label), box);
		l->setBuddy(buddy);
		m_form->addRow(l, field);
	};
	addRow("unlock.file", pathRow, m_path);
	addRow("unlock.password", m_password, m_password);
	addRow("unlock.password2", m_password2, m_password2);
	m_form->addRow(QString(), m_useKey);
	addRow("unlock.keyfile", m_keyRow, m_keyFile);
	m_form->addRow(QString(), m_makeKey);
	m_form->addRow(QString(), m_remember);
	m_form->addRow(QString(), m_strength);
	v->addLayout(m_form);
	m_message = new QLabel(box);
	m_message->setWordWrap(true);
	v->addWidget(m_message);
	auto* buttons = new QHBoxLayout;
	m_switch = new QPushButton(box);
	m_switch->setFlat(true);
	buttons->addWidget(m_switch);
	buttons->addStretch(1);
	m_cancel = new QPushButton(rjText(backend, "unlock.cancel"), box);
	m_ok = new QPushButton(rjText(backend, "unlock.button"), box);
	m_ok->setDefault(true);
	buttons->addWidget(m_cancel);
	buttons->addWidget(m_ok);
	v->addLayout(buttons);
	connect(m_ok, &QPushButton::clicked, this, &UnlockOverlay::submit);
	connect(m_password, &QLineEdit::returnPressed, this, &UnlockOverlay::submit);
	connect(m_password2, &QLineEdit::returnPressed, this, &UnlockOverlay::submit);
	connect(m_cancel, &QPushButton::clicked, this, &QWidget::hide);
	connect(m_browse, &QPushButton::clicked, this, &UnlockOverlay::browse);
	connect(m_browseKey, &QPushButton::clicked, this, &UnlockOverlay::browseKeyFile);
	connect(m_useKey, &QCheckBox::toggled, this, [this](bool on) {
		updateKeyRows();
		if (on)
			m_keyFile->setFocus();
	});
	connect(m_switch, &QPushButton::clicked, this, [this] { setMode(!m_create); });
	connect(m_password, &QLineEdit::textChanged, this, &UnlockOverlay::updateStrength);
	connect(m_password2, &QLineEdit::textChanged, this, &UnlockOverlay::updateStrength);
	connect(m_makeKey, &QCheckBox::toggled, this, [this](bool on) {
		// Vorschlag für den Ort der neuen Schlüsseldatei: neben der KDBX, Endung .key.
		if (on && m_keyFile->text().trimmed().isEmpty() && !m_path->text().trimmed().isEmpty())
		{
			const QFileInfo fi(m_path->text().trimmed());
			m_keyFile->setText(fi.dir().filePath(fi.completeBaseName() + QStringLiteral(".key")));
		}
	});
	setMode(false);
	if (parent)
		parent->installEventFilter(this);
	hide();
}

void UnlockOverlay::setMode(bool create)
{
	m_create = create;
	m_title->setText(rjText(m_backend, create ? "unlock.title.new" : "unlock.title"));
	m_ok->setText(rjText(m_backend, create ? "unlock.button.new" : "unlock.button"));
	m_switch->setText(rjText(m_backend, create ? "unlock.switch.open" : "unlock.switch.new"));
	m_form->setRowVisible(m_password2, create);
	m_form->setRowVisible(m_strength, create);
	m_keyFile->setPlaceholderText(create ? rjText(m_backend, "unlock.keyfile.hint.new") : QString());
	m_message->clear();
	m_password->clear();
	m_password2->clear();
	m_makeKey->setChecked(false);
	if (create)
	{
		m_useKey->setChecked(false);
		m_keyFile->clear();
	}
	else
		prefillKeyFile();
	updateKeyRows();
	m_hint->setText(create ? rjText(m_backend, "unlock.hint.new") : QString());
	m_hint->setVisible(create);
	updateStrength();
}

void UnlockOverlay::open(const QString& path, bool locked)
{
	m_locked = locked;
	setMode(false);
	m_switch->setVisible(!locked);
	if (!path.isEmpty())
	{
		m_path->setText(path);
		prefillKeyFile();
	}
	else if (!locked)
	{
		// Erster Start ohne bekannte KDBX: Öffnen oder neu anlegen anbieten.
		m_hint->setText(rjText(m_backend, "unlock.hint.first"));
		m_hint->show();
	}
	m_cancel->setVisible(!locked);
	m_message->clear();
	setBusy(false);
	setGeometry(parentWidget()->rect());
	raise();
	show();
	(m_path->text().isEmpty() ? m_path : m_password)->setFocus();
}

void UnlockOverlay::openNew()
{
	m_locked = false;
	setMode(true);
	m_switch->setVisible(true);
	m_cancel->setVisible(true);
	if (m_path->text().trimmed().isEmpty() || QFileInfo::exists(m_path->text().trimmed()))
	{
		// Vorschlag: der eine Ordner von remotejoe (~/remotejoe); bei alter Installation die Dokumente.
		const QString home = m_backend->homeDir();
		const QString base = home.isEmpty() ? QStandardPaths::writableLocation(QStandardPaths::DocumentsLocation) : home;
		m_path->setText(QDir(base).filePath(QStringLiteral("remotejoe.kdbx")));
	}
	m_useKey->setChecked(false);
	m_keyFile->clear();
	setBusy(false);
	setGeometry(parentWidget()->rect());
	raise();
	show();
	m_password->setFocus();
}

void UnlockOverlay::updateKeyRows()
{
	const bool on = m_useKey->isChecked();
	m_form->setRowVisible(m_keyRow, on);
	m_form->setRowVisible(m_makeKey, m_create && on);
	if (!on)
		m_makeKey->setChecked(false);
}

void UnlockOverlay::prefillKeyFile()
{
	const QString kf = m_backend->keyFileFor(m_path->text().trimmed());
	m_keyFile->setText(kf);
	m_useKey->setChecked(!kf.isEmpty());
	updateKeyRows();
}

void UnlockOverlay::browseKeyFile()
{
	const QFileInfo db(m_path->text().trimmed());
	const QString start = m_keyFile->text().trimmed().isEmpty() ? db.absolutePath() : m_keyFile->text().trimmed();
	const QString title = rjText(m_backend, "unlock.browse.key");
	const QString f = (m_create && m_makeKey->isChecked())
	                    ? QFileDialog::getSaveFileName(this, title, start, QString(), nullptr, QFileDialog::DontConfirmOverwrite)
	                    : QFileDialog::getOpenFileName(this, title, start);
	if (!f.isEmpty())
		m_keyFile->setText(f);
}

void UnlockOverlay::placeLogo()
{
	// Groß, aber nie über die Eingabebox: bei schmalen Fenstern ausblenden.
	const int side = qBound(128, qMin(width(), height()) / 3, 320);
	const int margin = 32;
	if (width() < 1000 || height() < 560)
	{
		m_logo->hide();
		return;
	}
	m_logo->setPixmap(QIcon(QStringLiteral(":/logo/remotejoe-512.png")).pixmap(side, side));
	m_logo->setGeometry(width() - side - margin, height() - side - margin, side, side);
	m_logo->lower();
	m_logo->show();
}

void UnlockOverlay::resizeEvent(QResizeEvent* e)
{
	QWidget::resizeEvent(e);
	placeLogo();
}

void UnlockOverlay::setBusy(bool busy, const QString& message)
{
	m_ok->setEnabled(!busy);
	m_password->setEnabled(!busy);
	m_password2->setEnabled(!busy);
	m_browse->setEnabled(!busy);
	m_useKey->setEnabled(!busy);
	m_keyFile->setEnabled(!busy);
	m_browseKey->setEnabled(!busy);
	m_switch->setEnabled(!busy);
	if (!message.isEmpty() || !busy)
		m_message->setText(message);
}

void UnlockOverlay::browse()
{
	const QString homeDir = m_backend->homeDir();
	const QString start = m_path->text().trimmed().isEmpty() ? (homeDir.isEmpty() ? QStandardPaths::writableLocation(QStandardPaths::DocumentsLocation) : homeDir) : m_path->text().trimmed();
	const QString filter = rjText(m_backend, "unlock.filter");
	if (m_create)
	{
		QString f = QFileDialog::getSaveFileName(this, rjText(m_backend, "unlock.browse.new"), start, filter, nullptr, QFileDialog::DontConfirmOverwrite);
		if (f.isEmpty())
			return;
		if (!f.endsWith(QStringLiteral(".kdbx"), Qt::CaseInsensitive))
			f += QStringLiteral(".kdbx");
		m_path->setText(f);
	}
	else
	{
		const QString f = QFileDialog::getOpenFileName(this, rjText(m_backend, "unlock.browse.open"), start, filter);
		if (!f.isEmpty())
			m_path->setText(f);
	}
}

void UnlockOverlay::updateStrength()
{
	if (!m_create)
		return;
	const QString p = m_password->text();
	QString t;
	if (!p.isEmpty())
	{
		if (!m_password2->text().isEmpty() && p != m_password2->text())
			t = rjText(m_backend, "unlock.err.mismatch");
		else if (p.size() < 12)
			t = rjText(m_backend, "unlock.strength.short");
		else
			t = rjText(m_backend, "unlock.strength.ok");
	}
	m_strength->setText(t);
}

void UnlockOverlay::submit()
{
	if (m_create)
	{
		const QString path = m_path->text().trimmed();
		QString pw = m_password->text();
		QString pw2 = m_password2->text();
		const bool useKey = m_useKey->isChecked();
		QString kf = useKey ? m_keyFile->text().trimmed() : QString();
		const bool make = useKey && m_makeKey->isChecked();
		auto wipe = [&] {
			pw.fill(QLatin1Char(' '));
			pw2.fill(QLatin1Char(' '));
		};
		if (path.isEmpty())
			m_message->setText(rjText(m_backend, "unlock.err.path"));
		else if (QFileInfo::exists(path))
			m_message->setText(rjText(m_backend, "unlock.err.exists"));
		else if (pw != pw2)
			m_message->setText(rjText(m_backend, "unlock.err.mismatch"));
		else if (pw.isEmpty() && kf.isEmpty() && !make)
			m_message->setText(rjText(m_backend, "unlock.err.empty"));
		else if (useKey && !make && kf.isEmpty())
			m_message->setText(rjText(m_backend, "unlock.err.keyfile.empty"));
		else if (useKey && !make && !QFileInfo(kf).isFile())
		{
			// Der Text wird nicht wiederholt: Dort steht manchmal versehentlich das Kennwort.
			m_keyFile->clear();
			m_message->setText(rjText(m_backend, "unlock.err.keyfile.missing"));
		}
		else
		{
			if (make && kf.isEmpty())
			{
				const QFileInfo fi(path);
				kf = fi.dir().filePath(fi.completeBaseName() + QStringLiteral(".key"));
			}
			setBusy(true, rjText(m_backend, "unlock.busy.new"));
			emit createRequested(path, pw, kf, make, m_remember->isChecked());
			m_password->clear();
			m_password2->clear();
		}
		wipe();
		return;
	}
	const QString kf = m_useKey->isChecked() ? m_keyFile->text().trimmed() : QString();
	if (m_useKey->isChecked() && kf.isEmpty())
	{
		m_message->setText(rjText(m_backend, "unlock.err.keyfile.empty"));
		return;
	}
	if (!kf.isEmpty() && !QFileInfo(kf).isFile())
	{
		// Der Text wird nicht wiederholt: Dort steht manchmal versehentlich das Kennwort.
		m_keyFile->clear();
		m_message->setText(rjText(m_backend, "unlock.err.keyfile.missing"));
		return;
	}
	QString pw = m_password->text();
	setBusy(true, rjText(m_backend, "unlock.busy"));
	emit unlockRequested(m_path->text(), pw, kf, m_remember->isChecked());
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

// ---------------------------------------------------------------- BulkAddTab

BulkAddTab::BulkAddTab(RjBackend* backend, const QString& parentId, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_parent(parentId)
{
	setProperty("editor", true);
	setObjectName(QStringLiteral("bulkAddTab"));
	const QJsonObject info = parse(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), parentId } }));
	if (m_parent.isEmpty())
		m_parent = info.value(QStringLiteral("id")).toString();
	const QJsonObject cred = info.value(QStringLiteral("credential")).toObject();
	auto* outer = new QVBoxLayout(this);
	QString folderName = info.value(QStringLiteral("name")).toString();
	if (folderName.isEmpty())
		folderName = rjText(backend, "bulk.root");
	auto* head = new QLabel(rjText(backend, "bulk.heading").arg(folderName), this);
	head->setObjectName(QStringLiteral("heading"));
	outer->addWidget(head);
	auto* split = new QSplitter(Qt::Vertical, this);
	outer->addWidget(split, 1);

	auto* left = new QWidget(split);
	auto* top = new QHBoxLayout(left);
	auto* hostsCol = new QVBoxLayout;
	auto* f = new QFormLayout;
	top->addLayout(hostsCol, 2);
	top->addLayout(f, 3);
	m_kind = new QComboBox(left);
	m_kind->setObjectName(QStringLiteral("bulkKind"));
	for (const char* k : { "rdp", "ssh", "vnc", "web", "proxmox", "hyperv", "psrp" })
		m_kind->addItem(rjText(backend, (QByteArray("add.") + k).constData()).remove(QLatin1Char('&')), QString::fromLatin1(k));
	auto* kl = new QLabel(rjText(backend, "bulk.kind"), left);
	kl->setBuddy(m_kind);
	f->addRow(kl, m_kind);
	m_hosts = new QPlainTextEdit(left);
	m_hosts->setObjectName(QStringLiteral("bulkHosts"));
	m_hosts->setTabChangesFocus(true);
	m_hosts->setPlaceholderText(rjText(backend, "bulk.hosts.placeholder"));
	m_hosts->setAccessibleName(rjText(backend, "bulk.hosts").remove(QLatin1Char('&')));
	auto* hl = new QLabel(rjText(backend, "bulk.hosts"), left);
	hl->setBuddy(m_hosts);
	hostsCol->addWidget(hl);
	hostsCol->addWidget(m_hosts, 1);
	m_pattern = new QLineEdit(QStringLiteral("{host}"), left);
	m_pattern->setObjectName(QStringLiteral("bulkPattern"));
	m_pattern->setToolTip(rjText(backend, "bulk.pattern.tip"));
	auto* pl = new QLabel(rjText(backend, "bulk.pattern"), left);
	pl->setBuddy(m_pattern);
	f->addRow(pl, m_pattern);
	m_patternError = errorLabel(left);
	f->addRow(QString(), m_patternError);
	m_template = new QComboBox(left);
	m_template->setObjectName(QStringLiteral("bulkTemplate"));
	auto* tl = new QLabel(rjText(backend, "bulk.template"), left);
	tl->setBuddy(m_template);
	f->addRow(tl, m_template);

	auto* credBox = new QWidget(left);
	auto* cv = new QVBoxLayout(credBox);
	cv->setContentsMargins(0, 0, 0, 0);
	m_credMode = new QButtonGroup(credBox);
	const char* modes[][2] = { { "inherit", "edit.cred.inherit" }, { "none", "edit.cred.none" }, { "direct", "bulk.cred.direct" },
		                       { "entry", "edit.cred.entry" },     { "named", "edit.cred.named" } };
	int id = 0;
	for (auto& m : modes)
	{
		auto* r = new QRadioButton(rjText(backend, m[1]), credBox);
		r->setProperty("mode", QString::fromLatin1(m[0]));
		r->setObjectName(QStringLiteral("bulk_cred_") + QString::fromLatin1(m[0]));
		m_credMode->addButton(r, id++);
		cv->addWidget(r);
		const QString mode = QString::fromLatin1(m[0]);
		if (mode == QLatin1String("direct"))
		{
			m_directBox = new QWidget(credBox);
			auto* df = new QFormLayout(m_directBox);
			df->setContentsMargins(24, 0, 0, 0);
			m_user = new QLineEdit(m_directBox);
			m_password = new QLineEdit(m_directBox);
			m_password->setEchoMode(QLineEdit::Password);
			auto* ul = new QLabel(rjText(backend, "edit.cred.username"), m_directBox);
			ul->setBuddy(m_user);
			auto* pwl = new QLabel(rjText(backend, "edit.cred.password"), m_directBox);
			pwl->setBuddy(m_password);
			df->addRow(ul, m_user);
			df->addRow(pwl, m_password);
			cv->addWidget(m_directBox);
		}
		else if (mode == QLatin1String("entry"))
		{
			m_entryBox = new QWidget(credBox);
			auto* ef = new QFormLayout(m_entryBox);
			ef->setContentsMargins(24, 0, 0, 0);
			m_entry = new QComboBox(m_entryBox);
			for (const QJsonValue& e : cred.value(QStringLiteral("entries")).toArray())
			{
				const QJsonObject o = e.toObject();
				m_entry->addItem(o.value(QStringLiteral("title")).toString(), o.value(QStringLiteral("uuid")).toString());
			}
			auto* el = new QLabel(rjText(backend, "edit.cred.entry.label"), m_entryBox);
			el->setBuddy(m_entry);
			ef->addRow(el, m_entry);
			cv->addWidget(m_entryBox);
		}
		else if (mode == QLatin1String("named"))
		{
			m_namedBox = new QWidget(credBox);
			auto* nf = new QFormLayout(m_namedBox);
			nf->setContentsMargins(24, 0, 0, 0);
			m_named = new QComboBox(m_namedBox);
			m_named->setEditable(true);
			for (const QJsonValue& n : cred.value(QStringLiteral("names")).toArray())
				m_named->addItem(n.toString(), n.toString());
			auto* nl = new QLabel(rjText(backend, "edit.cred.named.label"), m_namedBox);
			nl->setBuddy(m_named);
			nf->addRow(nl, m_named);
			cv->addWidget(m_namedBox);
		}
	}
	m_credMode->button(0)->setChecked(true);
	if (!info.value(QStringLiteral("unlocked")).toBool())
		for (int i : { 2, 3 })
			m_credMode->button(i)->setEnabled(false);
	f->addRow(rjText(backend, "edit.cat.credentials"), credBox);

	auto* right = new QWidget(split);
	auto* rv = new QVBoxLayout(right);
	auto* prevLabel = new QLabel(rjText(backend, "bulk.preview"), right);
	m_preview = new QTableWidget(0, 4, right);
	m_preview->setObjectName(QStringLiteral("bulkPreview"));
	m_preview->setHorizontalHeaderLabels({ rjText(backend, "bulk.col.name"), rjText(backend, "bulk.col.host"), rjText(backend, "bulk.col.port"),
	                                       rjText(backend, "bulk.col.hints") });
	for (int c = 0; c < 3; ++c)
		m_preview->horizontalHeader()->setSectionResizeMode(c, QHeaderView::ResizeToContents);
	m_preview->horizontalHeader()->setSectionResizeMode(3, QHeaderView::Stretch);
	m_preview->setEditTriggers(QAbstractItemView::NoEditTriggers);
	m_preview->setSelectionBehavior(QAbstractItemView::SelectRows);
	m_preview->setWordWrap(true);
	m_preview->verticalHeader()->setVisible(true);
	m_preview->setAccessibleName(rjText(backend, "bulk.preview").remove(QLatin1Char('&')));
	prevLabel->setBuddy(m_preview);
	m_summary = new QLabel(right);
	m_summary->setObjectName(QStringLiteral("bulkSummary"));
	m_summary->setWordWrap(true);
	auto* buttons = new QHBoxLayout;
	m_create = new QPushButton(rjText(backend, "bulk.create"), right);
	m_create->setObjectName(QStringLiteral("bulkCreate"));
	m_create->setDefault(true);
	buttons->addStretch(1);
	buttons->addWidget(m_create);
	rv->addWidget(prevLabel);
	rv->addWidget(m_preview, 1);
	rv->addWidget(m_summary);
	rv->addLayout(buttons);
	split->setStretchFactor(0, 2);
	split->setStretchFactor(1, 3);

	m_timer = new QTimer(this);
	m_timer->setSingleShot(true);
	m_timer->setInterval(250);
	connect(m_timer, &QTimer::timeout, this, &BulkAddTab::refresh);
	auto later = [this] { m_timer->start(); };
	connect(m_hosts, &QPlainTextEdit::textChanged, this, later);
	connect(m_pattern, &QLineEdit::textChanged, this, later);
	connect(m_kind, &QComboBox::currentIndexChanged, this, [this] {
		fillTemplates();
		refresh();
	});
	connect(m_template, &QComboBox::currentIndexChanged, this, later);
	connect(m_credMode, &QButtonGroup::idToggled, this, [this](int, bool on) {
		if (on)
			updateCredentialMode();
	});
	connect(m_create, &QPushButton::clicked, this, &BulkAddTab::create);
	auto* sc = new QShortcut(QKeySequence(Qt::CTRL | Qt::Key_Return), this);
	connect(sc, &QShortcut::activated, this, &BulkAddTab::create);
	fillTemplates();
	updateCredentialMode();
	refresh();
}

QString BulkAddTab::edit(const QJsonObject& cmd) const
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

void BulkAddTab::fillTemplates()
{
	const QString kind = m_kind->currentData().toString();
	const QSignalBlocker block(m_template);
	m_template->clear();
	m_template->addItem(rjText(m_backend, "bulk.template.default"), QString());
	for (const QJsonValue& v : QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("templates") } }).toUtf8()).array())
	{
		const QJsonObject o = v.toObject();
		if (o.value(QStringLiteral("protocol")).toString() == kind)
			m_template->addItem(o.value(QStringLiteral("name")).toString() + (o.value(QStringLiteral("is_default")).toBool() ? QStringLiteral(" ★") : QString()),
			                    o.value(QStringLiteral("id")).toString());
	}
}

void BulkAddTab::updateCredentialMode()
{
	const QString mode = m_credMode->checkedButton() ? m_credMode->checkedButton()->property("mode").toString() : QString();
	m_directBox->setVisible(mode == QLatin1String("direct"));
	m_entryBox->setVisible(mode == QLatin1String("entry"));
	m_namedBox->setVisible(mode == QLatin1String("named"));
}

QJsonObject BulkAddTab::command(const QString& op) const
{
	QJsonObject c{ { QStringLiteral("op"), op },
		           { QStringLiteral("parent"), m_parent },
		           { QStringLiteral("kind"), m_kind->currentData().toString() },
		           { QStringLiteral("text"), m_hosts->toPlainText() },
		           { QStringLiteral("pattern"), m_pattern->text() } };
	if (!m_template->currentData().toString().isEmpty())
		c.insert(QStringLiteral("template"), m_template->currentData().toString());
	return c;
}

void BulkAddTab::refresh()
{
	m_timer->stop();
	const QJsonObject r = parse(edit(command(QStringLiteral("bulk_preview"))));
	m_preview->setRowCount(0);
	if (r.contains(QStringLiteral("error")))
	{
		setWarn(m_patternError, r.value(QStringLiteral("error")).toString());
		m_summary->clear();
		m_create->setEnabled(false);
		return;
	}
	setWarn(m_patternError, QString());
	const QJsonArray rows = r.value(QStringLiteral("rows")).toArray();
	m_preview->setRowCount(rows.size());
	QStringList lines;
	for (int i = 0; i < rows.size(); ++i)
	{
		const QJsonObject o = rows[i].toObject();
		const bool valid = o.value(QStringLiteral("valid")).toBool();
		QStringList notes;
		for (const QJsonValue& e : o.value(QStringLiteral("errors")).toArray())
			notes << QStringLiteral("✗ ") + e.toString();
		for (const QJsonValue& h : o.value(QStringLiteral("hints")).toArray())
			notes << QStringLiteral("ℹ ") + h.toString();
		if (notes.isEmpty())
			notes << rjText(m_backend, "bulk.row.ok");
		const QString cells[] = { o.value(QStringLiteral("name")).toString(), o.value(QStringLiteral("host")).toString(),
			                      QString::number(o.value(QStringLiteral("port")).toInt()), notes.join(QLatin1Char('\n')) };
		for (int c = 0; c < 4; ++c)
		{
			auto* it = new QTableWidgetItem(cells[c]);
			if (!valid)
			{
				QFont fnt = it->font();
				fnt.setBold(c == 3);
				it->setFont(fnt);
			}
			m_preview->setItem(i, c, it);
		}
		m_preview->setVerticalHeaderItem(i, new QTableWidgetItem(QString::number(o.value(QStringLiteral("line")).toInt())));
	}
	m_preview->resizeRowsToContents();
	const int valid = r.value(QStringLiteral("valid")).toInt();
	const int invalid = r.value(QStringLiteral("invalid")).toInt();
	m_summary->setText(rjText(m_backend, "bulk.summary").arg(valid).arg(invalid).arg(r.value(QStringLiteral("limit")).toInt()));
	m_create->setEnabled(valid > 0);
}

void BulkAddTab::create()
{
	refresh();
	if (!m_create->isEnabled())
		return;
	QJsonObject c = command(QStringLiteral("bulk_add"));
	const QString mode = m_credMode->checkedButton()->property("mode").toString();
	QJsonObject cred{ { QStringLiteral("mode"), mode } };
	if (mode == QLatin1String("direct"))
	{
		cred.insert(QStringLiteral("username"), m_user->text());
		// Kennwort byte-genau weitergeben; das Feld wird danach geleert.
		cred.insert(QStringLiteral("password"), m_password->text());
	}
	else if (mode == QLatin1String("entry"))
		cred.insert(QStringLiteral("entry"), m_entry->currentData().toString());
	else if (mode == QLatin1String("named"))
		cred.insert(QStringLiteral("name"), m_named->currentText());
	c.insert(QStringLiteral("credential"), cred);
	QString raw = QString::fromUtf8(QJsonDocument(c).toJson(QJsonDocument::Compact));
	const QJsonObject r = parse(m_backend->edit(raw));
	raw.fill(QLatin1Char(' '));
	if (r.contains(QStringLiteral("error")))
	{
		m_summary->setText(rjText(m_backend, "bulk.failed").arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	wipeSecrets();
	const int created = r.value(QStringLiteral("created")).toArray().size();
	// Ungültige Zeilen bleiben stehen, die Vorschau zeigt ihre Fehler.
	m_hosts->setPlainText(r.value(QStringLiteral("remaining")).toString());
	refresh();
	const QJsonObject saved = parse(edit({ { QStringLiteral("op"), QStringLiteral("save") } }));
	const QString msg = rjText(m_backend, "bulk.done").arg(created);
	m_summary->setText(saved.value(QStringLiteral("saved")).toBool() ? msg : msg + QStringLiteral(" ") + rjText(m_backend, "bulk.not_saved"));
	emit indexChanged();
	emit status(msg);
}

void BulkAddTab::wipeSecrets()
{
	QString t = m_password->text();
	t.fill(QLatin1Char(' '));
	m_password->clear();
}

// ---------------------------------------------------------------- SecurityBar

SecurityBar::SecurityBar(RjBackend* backend, QWidget* parent)
  : QFrame(parent)
  , m_backend(backend)
{
	setObjectName(QStringLiteral("securityBar"));
	setFrameShape(QFrame::StyledPanel);
	auto* lay = new QVBoxLayout(this);
	lay->setContentsMargins(8, 6, 8, 6);
	m_title = new QLabel(this);
	m_title->setObjectName(QStringLiteral("securityTitle"));
	m_title->setWordWrap(true);
	m_details = new QLabel(this);
	m_details->setObjectName(QStringLiteral("securityDetails"));
	m_details->setWordWrap(true);
	m_details->setTextFormat(Qt::RichText);
	m_details->setTextInteractionFlags(Qt::TextSelectableByMouse | Qt::TextSelectableByKeyboard);
	m_details->setFocusPolicy(Qt::StrongFocus);
	m_checked = new QCheckBox(rjText(backend, "security.checked"), this);
	m_checked->setObjectName(QStringLiteral("securityChecked"));
	m_countdown = new QLabel(this);
	auto* buttons = new QHBoxLayout;
	m_reject = new QPushButton(rjText(backend, "security.reject"), this);
	m_reject->setObjectName(QStringLiteral("securityReject"));
	m_reject->setDefault(true);
	m_reject->setAutoDefault(true);
	m_accept = new QPushButton(rjText(backend, "security.accept"), this);
	m_accept->setObjectName(QStringLiteral("securityAccept"));
	m_accept->setAutoDefault(false);
	// Knöpfe links unter dem Text: Sie bleiben sichtbar, auch wenn der Tab schmal ist.
	buttons->addWidget(m_accept);
	buttons->addWidget(m_reject);
	buttons->addWidget(m_countdown, 1);
	lay->addWidget(m_title);
	lay->addWidget(m_details);
	lay->addWidget(m_checked);
	lay->addLayout(buttons);
	m_timer = new QTimer(this);
	m_timer->setInterval(1000);
	connect(m_timer, &QTimer::timeout, this, &SecurityBar::tick);
	connect(m_reject, &QPushButton::clicked, this, [this] { finish(false); });
	connect(m_accept, &QPushButton::clicked, this, [this] { finish(true); });
	connect(m_checked, &QCheckBox::toggled, m_accept, &QPushButton::setEnabled);
	hide();
}

void SecurityBar::showQuestion(int q, const QJsonObject& e, std::function<void(bool)> done)
{
	m_queue.append({ q, e, std::move(done) });
	if (m_current < 0)
		next();
}

void SecurityBar::next()
{
	if (m_queue.isEmpty())
	{
		m_current = -1;
		m_timer->stop();
		hide();
		return;
	}
	const Pending p = m_queue.takeFirst();
	m_current = p.q;
	m_done = p.done;
	const QJsonObject info = p.e.value(QStringLiteral("info")).toObject();
	const QString previous = info.value(QStringLiteral("previous")).toString();
	const bool changed = !previous.isEmpty();
	setProperty("changed", changed);
	style()->unpolish(this);
	style()->polish(this);
	m_title->setText((changed ? QStringLiteral("⚠ ") : QString()) + p.e.value(QStringLiteral("text")).toString());
	const QString fp = info.value(QStringLiteral("fingerprint")).toString();
	QString html;
	auto row = [&](const char* label, const QString& value, bool mono) {
		html += QStringLiteral("<tr><td style='padding-right:12px'>%1</td><td>%2</td></tr>")
		          .arg(rjText(m_backend, label).toHtmlEscaped(),
		               mono ? QStringLiteral("<code>") + value.toHtmlEscaped() + QStringLiteral("</code>") : value.toHtmlEscaped());
	};
	// Lange Fingerabdrücke in Zeilen umbrechen (16 Bytes bzw. 32 Zeichen je Zeile), damit die Leiste
	// nie breiter als der Tab wird.
	auto wrapped = [](const QString& v) {
		QStringList lines;
		const QStringList parts = v.split(QLatin1Char(':'));
		if (parts.size() > 16)
			for (int i = 0; i < parts.size(); i += 16)
				lines << parts.mid(i, 16).join(QLatin1Char(':'));
		else
			for (int i = 0; i < v.size(); i += 32)
				lines << v.mid(i, 32);
		QStringList esc;
		for (const QString& l : lines)
			esc << l.toHtmlEscaped();
		return QStringLiteral("<code>") + esc.join(QStringLiteral("<br>")) + QStringLiteral("</code>");
	};
	auto fpRow = [&](const char* label, const QString& value) {
		html += QStringLiteral("<tr><td style='padding-right:12px' valign='top'>%1</td><td>%2</td></tr>").arg(rjText(m_backend, label).toHtmlEscaped(), wrapped(value));
	};
	if (!fp.isEmpty())
	{
		const QString host = info.value(QStringLiteral("host")).toString();
		const int port = info.value(QStringLiteral("port")).toInt();
		if (!host.isEmpty())
			row("security.host", port > 0 ? host + QLatin1Char(':') + QString::number(port) : host, false);
		row("security.key_type", info.value(QStringLiteral("key_type")).toString(), true);
		fpRow(changed ? "security.fingerprint.new" : "security.fingerprint", fp);
		if (changed)
			fpRow("security.fingerprint.old", previous);
		html = QStringLiteral("<table>") + html + QStringLiteral("</table>");
	}
	else
		html = p.e.value(QStringLiteral("detail")).toString().toHtmlEscaped().replace(QLatin1Char('\n'), QStringLiteral("<br>"));
	m_details->setText(html);
	m_details->setAccessibleDescription(p.e.value(QStringLiteral("detail")).toString());
	// Geänderter Schlüssel: Annehmen erst nach ausdrücklicher Bestätigung.
	m_checked->setChecked(false);
	m_checked->setVisible(changed);
	m_accept->setEnabled(!changed);
	m_left = p.e.value(QStringLiteral("timeout_secs")).toInt();
	tick();
	if (m_left > 0)
		m_timer->start();
	show();
	m_reject->setFocus();
}

void SecurityBar::tick()
{
	if (m_current < 0)
		return;
	if (m_timer->isActive())
		--m_left;
	if (m_left <= 0 && m_timer->isActive())
	{
		finish(false);
		return;
	}
	m_countdown->setText(m_left > 0 ? rjText(m_backend, "security.countdown").arg(m_left) : QString());
}

void SecurityBar::finish(bool accept)
{
	auto done = m_done;
	m_done = nullptr;
	m_current = -1;
	m_timer->stop();
	if (done)
		done(accept);
	next();
}

void SecurityBar::closeQuestion(int q, const QString& reason)
{
	for (int i = 0; i < m_queue.size(); ++i)
		if (m_queue[i].q == q)
		{
			m_queue.removeAt(i);
			return;
		}
	if (q != m_current)
		return;
	m_done = nullptr;
	m_current = -1;
	m_timer->stop();
	emit closedWithoutAnswer(reason);
	next();
}

void SecurityBar::rejectAll()
{
	while (m_current >= 0)
		finish(false);
}

void SecurityBar::keyPressEvent(QKeyEvent* e)
{
	if (e->key() == Qt::Key_Escape && m_current >= 0)
	{
		finish(false);
		return;
	}
	QFrame::keyPressEvent(e);
}

// ----------------------------------------------------------- ConnectPromptTab

ConnectPromptTab::ConnectPromptTab(RjBackend* backend, const QString& targetId, const QString& title, QWidget* parent)
  : ToolTab(parent)
  , m_target(targetId)
  , m_title(title)
{
	setObjectName(QStringLiteral("connectPromptTab"));
	auto* lay = new QVBoxLayout(this);
	auto* head = new QLabel(rjText(backend, "prompt.heading").arg(title), this);
	head->setObjectName(QStringLiteral("heading"));
	m_message = new QLabel(this);
	m_message->setWordWrap(true);
	m_message->hide();
	m_bar = new ActionBar(backend, this);
	m_security = new SecurityBar(backend, this);
	lay->addWidget(head);
	lay->addWidget(m_message);
	lay->addWidget(m_bar);
	lay->addWidget(m_security);
	lay->addStretch(1);
}

void ConnectPromptTab::setMessage(const QString& text)
{
	m_message->setText(text);
	m_message->setVisible(!text.isEmpty());
}

void ConnectPromptTab::wipeSecrets()
{
	m_security->rejectAll();
	m_bar->dismiss();
}

// ------------------------------------------------------------------ ImportTab

namespace {

// Aufklappbarer Bereich: Kopfzeile mit Pfeil, darunter der Inhalt.
QWidget* makeFold(const QString& title, QWidget* body, bool open, QWidget* parent)
{
	auto* box = new QWidget(parent);
	auto* l = new QVBoxLayout(box);
	l->setContentsMargins(0, 0, 0, 0);
	l->setSpacing(2);
	auto* btn = new QToolButton(box);
	btn->setText(title);
	btn->setCheckable(true);
	btn->setChecked(open);
	btn->setAutoRaise(true);
	btn->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
	btn->setArrowType(open ? Qt::DownArrow : Qt::RightArrow);
	QFont f = btn->font();
	f.setBold(true);
	btn->setFont(f);
	body->setParent(box);
	body->setVisible(open);
	QObject::connect(btn, &QToolButton::toggled, body, [btn, body](bool on) {
		body->setVisible(on);
		btn->setArrowType(on ? Qt::DownArrow : Qt::RightArrow);
	});
	l->addWidget(btn, 0, Qt::AlignLeft);
	l->addWidget(body);
	return box;
}

QListWidget* makeList(const QStringList& lines, const QString& accessibleName, QWidget* parent)
{
	auto* list = new QListWidget(parent);
	list->addItems(lines);
	list->setFixedHeight(qMin(220, list->sizeHintForRow(0) * int(lines.size()) + 2 * list->frameWidth() + 4));
	list->setAccessibleName(accessibleName);
	list->setEditTriggers(QAbstractItemView::NoEditTriggers);
	list->setWordWrap(true);
	return list;
}

} // namespace

ImportTab::ImportTab(RjBackend* backend, const QString& path, const QJsonObject& report, const QString& summary, QWidget* parent)
  : ToolTab(parent)
{
	setObjectName(QStringLiteral("importTab"));
	auto* lay = new QVBoxLayout(this);
	auto* head = new QLabel(rjText(backend, "import.heading").arg(path), this);
	head->setObjectName(QStringLiteral("heading"));
	head->setWordWrap(true);
	lay->addWidget(head);

	const auto num = [&report](const char* key) { return report.value(QLatin1String(key)).toInt(); };
	const auto strings = [](const QJsonArray& a) {
		QStringList out;
		for (const QJsonValue& v : a)
			out << v.toString();
		return out;
	};
	int withoutPassword = 0;

	if (report.isEmpty())
	{
		// Ohne strukturierten Bericht: der Text als Ganzes.
		auto* text = new QPlainTextEdit(summary, this);
		text->setObjectName(QStringLiteral("importSummary"));
		text->setReadOnly(true);
		text->setAccessibleName(rjText(backend, "import.summary"));
		lay->addWidget(text, 1);
	}
	else
	{
		// Zahlenkopf.
		auto* counts = new QLabel(rjText(backend, "import.counts").arg(num("folders")).arg(num("targets")).arg(num("credentials_in_source")).arg(num("credentials_unique")).arg(num("references")), this);
		counts->setObjectName(QStringLiteral("importCounts"));
		counts->setWordWrap(true);
		lay->addWidget(counts);

		// Hinweisleiste: Kennwörter, die nicht mitkommen, stehen ganz oben.
		const QJsonArray entries = report.value(QStringLiteral("encrypted_entries")).toArray();
		withoutPassword = num("encrypted_passwords");
		if (withoutPassword > 0)
		{
			auto* notice = new QFrame(this);
			notice->setObjectName(QStringLiteral("importNotice"));
			notice->setFrameShape(QFrame::StyledPanel);
			notice->setStyleSheet(QStringLiteral("QFrame#importNotice { background: #fff4ce; border: 1px solid #c79100; border-radius: 4px; } "
			                                     "QFrame#importNotice QLabel { color: #3d2c00; background: transparent; }"));
			auto* nl = new QVBoxLayout(notice);
			auto* title = new QLabel(rjText(backend, "import.notice.title").arg(withoutPassword), notice);
			QFont f = title->font();
			f.setBold(true);
			title->setFont(f);
			auto* body = new QLabel(rjText(backend, "import.notice.body"), notice);
			body->setWordWrap(true);
			nl->addWidget(title);
			nl->addWidget(body);
			notice->setAccessibleName(title->text());
			lay->addWidget(notice);
		}

		// Einklappbare Bereiche.
		QStringList passwordLines;
		for (const QJsonValue& v : entries)
		{
			const QJsonArray pair = v.toArray();
			const bool credential = pair.at(0).toString() == QLatin1String("credential");
			passwordLines << QStringLiteral("%1 „%2“").arg(rjText(backend, credential ? "import.kind.credential" : "import.kind.target"), pair.at(1).toString());
		}
		QStringList skipped;
		for (const QJsonValue& v : report.value(QStringLiteral("skipped")).toArray())
			skipped << QStringLiteral("%1: %2").arg(v.toArray().at(0).toString(), v.toArray().at(1).toString());
		const QStringList warnings = strings(report.value(QStringLiteral("warnings")).toArray());
		QStringList unknown;
		for (const QJsonValue& v : report.value(QStringLiteral("unknown_fields")).toArray())
			unknown << QStringLiteral("%1 (%2×)").arg(v.toArray().at(0).toString()).arg(v.toArray().at(1).toInt());

		struct Section { const char* key; QStringList lines; bool open; };
		const Section sections[] = {
			{ "import.fold.passwords", passwordLines, true },
			{ "import.fold.warnings", warnings, warnings.size() <= 5 },
			{ "import.fold.skipped", skipped, false },
			{ "import.fold.unknown", unknown, false },
		};
		for (const Section& sec : sections)
		{
			if (sec.lines.isEmpty())
				continue;
			const QString title = rjText(backend, sec.key).arg(sec.lines.size());
			lay->addWidget(makeFold(title, makeList(sec.lines, title, this), sec.open, this));
		}
		lay->addStretch(1);
	}

	auto* buttons = new QHBoxLayout;
	auto* apply = new QPushButton(rjText(backend, "import.apply"), this);
	apply->setObjectName(QStringLiteral("importApply"));
	auto* cancel = new QPushButton(rjText(backend, "bar.cancel"), this);
	if (withoutPassword > 0)
	{
		auto* hint = new QLabel(rjText(backend, "import.apply.hint").arg(withoutPassword), this);
		hint->setObjectName(QStringLiteral("importApplyHint"));
		buttons->addWidget(hint);
	}
	buttons->addStretch(1);
	buttons->addWidget(apply);
	buttons->addWidget(cancel);
	lay->addLayout(buttons);
	connect(apply, &QPushButton::clicked, this, [this, backend] {
		emit status(backend->importApply());
		emit indexChanged();
		emit closeRequested();
	});
	connect(cancel, &QPushButton::clicked, this, &ToolTab::closeRequested);
}

// ---------------------------------------------------------------- PatchdayTab

PatchdayTab::PatchdayTab(RjBackend* backend, const QStringList& ids, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_ids(ids)
{
	setObjectName(QStringLiteral("patchdayTab"));
	auto* lay = new QVBoxLayout(this);
	m_table = new QTableWidget(0, 5, this);
	m_table->setHorizontalHeaderLabels({ rjText(backend, "patch.col.target"), rjText(backend, "patch.col.state"), rjText(backend, "patch.col.count"),
	                                     rjText(backend, "patch.col.progress"), QString() });
	m_table->horizontalHeader()->setSectionResizeMode(0, QHeaderView::Stretch);
	m_table->horizontalHeader()->setSectionResizeMode(1, QHeaderView::Stretch);
	m_table->setSelectionBehavior(QAbstractItemView::SelectRows);
	m_table->setEditTriggers(QAbstractItemView::NoEditTriggers);
	m_table->verticalHeader()->setDefaultSectionSize(30);
	m_table->setAccessibleName(rjText(backend, "patch.title"));
	m_log = new QPlainTextEdit(this);
	m_log->setReadOnly(true);
	m_log->setMaximumHeight(160);
	m_log->setAccessibleName(rjText(backend, "patch.log"));
	m_bar = new ActionBar(backend, this);
	auto* buttons = new QHBoxLayout;
	auto* run = new QPushButton(rjText(backend, "patch.run"), this);
	run->setObjectName(QStringLiteral("patchRun"));
	auto* cancel = new QPushButton(rjText(backend, "patch.cancel_target"), this);
	buttons->addStretch(1);
	buttons->addWidget(run);
	buttons->addWidget(cancel);
	auto* hint = new QLabel(rjText(backend, "patch.dry_run"), this);
	hint->setWordWrap(true);
	lay->addWidget(hint);
	lay->addWidget(m_bar);
	lay->addWidget(m_table, 1);
	lay->addWidget(m_log);
	lay->addLayout(buttons);
	connect(run, &QPushButton::clicked, this, [this] {
		m_bar->ask(rjText(m_backend, "patch.run.confirm").arg(m_ids.size()), { rjText(m_backend, "patch.run.button") }, [this](int i) {
			if (i == 0)
				emit status(m_backend->startPatchday(QString::fromUtf8(QJsonDocument(QJsonArray::fromStringList(m_ids)).toJson(QJsonDocument::Compact)), true));
		});
	});
	auto cancelCurrent = [this] {
		const int r = m_table->currentRow();
		if (r >= 0)
			cancelRow(r);
	};
	connect(cancel, &QPushButton::clicked, this, cancelCurrent);
	auto* del = new QShortcut(QKeySequence(Qt::Key_Delete), m_table);
	del->setContext(Qt::WidgetShortcut);
	connect(del, &QShortcut::activated, this, cancelCurrent);
	connect(m_table, &QTableWidget::currentCellChanged, this, [this] {
		m_version = 0;
		refresh();
	});
	m_timer = new QTimer(this);
	connect(m_timer, &QTimer::timeout, this, &PatchdayTab::refresh);
	m_timer->start(250);
	refresh();
}

void PatchdayTab::cancelRow(int r)
{
	if (auto* it = m_table->item(r, 0))
		m_backend->cancelPatchTarget(it->data(Qt::UserRole).toString());
	m_version = 0;
	refresh();
}

void PatchdayTab::refresh()
{
	QElapsedTimer t;
	t.start();
	const QString selected =
	  m_table->currentRow() >= 0 && m_table->item(m_table->currentRow(), 0) ? m_table->item(m_table->currentRow(), 0)->data(Qt::UserRole).toString() : QString();
	QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("patchday") }, { QStringLiteral("since"), double(m_version) } };
	if (!selected.isEmpty())
		cmd.insert(QStringLiteral("selected"), selected);
	const QJsonObject v = parse(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))));
	if (v.value(QStringLiteral("unchanged")).toBool())
		return;
	m_version = qint64(v.value(QStringLiteral("version")).toDouble());
	const QJsonArray rows = v.value(QStringLiteral("rows")).toArray();
	if (m_table->rowCount() != rows.size())
		m_table->setRowCount(rows.size());
	for (int i = 0; i < rows.size(); ++i)
	{
		const QJsonObject r = rows[i].toObject();
		// Nur geänderte Zellen anfassen: ruhiges Bild auch bei vielen Zielen.
		auto set = [&](int col, const QString& text) {
			auto* it = m_table->item(i, col);
			if (!it)
			{
				it = new QTableWidgetItem;
				m_table->setItem(i, col, it);
			}
			if (it->text() != text)
				it->setText(text);
			return it;
		};
		set(0, r.value(QStringLiteral("name")).toString())->setData(Qt::UserRole, r.value(QStringLiteral("target")).toString());
		QString state = r.value(QStringLiteral("label")).toString();
		if (r.value(QStringLiteral("detail")).isString())
			state += QStringLiteral(": ") + r.value(QStringLiteral("detail")).toString();
		else if (r.value(QStringLiteral("last")).isString())
			state += QStringLiteral(" – ") + r.value(QStringLiteral("last")).toString();
		set(1, state);
		set(2, r.value(QStringLiteral("count")).isNull() ? QString() : QString::number(r.value(QStringLiteral("count")).toInt()));
		auto* bar = qobject_cast<QProgressBar*>(m_table->cellWidget(i, 3));
		if (!bar)
		{
			bar = new QProgressBar(m_table);
			m_table->setCellWidget(i, 3, bar);
		}
		if (bar->value() != r.value(QStringLiteral("percent")).toInt())
			bar->setValue(r.value(QStringLiteral("percent")).toInt());
		auto* stop = qobject_cast<QPushButton*>(m_table->cellWidget(i, 4));
		if (!stop)
		{
			stop = new QPushButton(rjText(m_backend, "bar.cancel"), m_table);
			stop->setAccessibleName(rjText(m_backend, "bar.cancel") + QStringLiteral(": ") + r.value(QStringLiteral("name")).toString());
			connect(stop, &QPushButton::clicked, this, [this, i] { cancelRow(i); });
			m_table->setCellWidget(i, 4, stop);
		}
		stop->setEnabled(!r.value(QStringLiteral("finished")).toBool());
		if (r.contains(QStringLiteral("log")))
		{
			QStringList log;
			for (const QJsonValue& l : r.value(QStringLiteral("log")).toArray())
				log << l.toString();
			if (m_log->toPlainText() != log.join(QLatin1Char('\n')))
				m_log->setPlainText(log.join(QLatin1Char('\n')));
		}
	}
	++refreshes;
	maxRefreshMs = qMax(maxRefreshMs, t.elapsed());
}

// ---------- CertCheckTab ----------

CertCheckTab::CertCheckTab(RjBackend* backend, const QStringList& ids, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_ids(ids)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setObjectName(QStringLiteral("certCheckTab"));
	auto* lay = new QVBoxLayout(this);
	auto* head = new QLabel(T("certcheck.title"), this);
	head->setObjectName(QStringLiteral("heading"));
	lay->addWidget(head);
	auto* info = new QLabel(T("certcheck.info"), this);
	info->setWordWrap(true);
	lay->addWidget(info);
	m_table = new QTableWidget(0, 6, this);
	m_table->setHorizontalHeaderLabels({ T("certcheck.col.target"), T("certcheck.col.protocol"), T("certcheck.col.address"), T("certcheck.col.kind"),
	                                     T("certcheck.col.fingerprint"), T("certcheck.col.state") });
	m_table->setEditTriggers(QAbstractItemView::NoEditTriggers);
	m_table->setSelectionBehavior(QAbstractItemView::SelectRows);
	m_table->verticalHeader()->hide();
	for (int c = 0; c < 4; ++c)
		m_table->horizontalHeader()->setSectionResizeMode(c, QHeaderView::ResizeToContents);
	m_table->horizontalHeader()->setSectionResizeMode(4, QHeaderView::Interactive);
	m_table->horizontalHeader()->setSectionResizeMode(5, QHeaderView::Stretch);
	m_table->setColumnWidth(4, 420);
	lay->addWidget(m_table, 1);
	m_state = new QLabel(this);
	m_state->setWordWrap(true);
	m_state->setTextInteractionFlags(Qt::TextSelectableByMouse | Qt::TextSelectableByKeyboard);
	lay->addWidget(m_state);
	auto* row = new QHBoxLayout;
	m_accept = new QPushButton(T("certcheck.accept"), this);
	m_again = new QPushButton(T("certcheck.again"), this);
	auto* close = new QPushButton(T("certcheck.close"), this);
	row->addWidget(m_accept);
	row->addWidget(m_again);
	row->addStretch(1);
	row->addWidget(close);
	lay->addLayout(row);
	connect(m_accept, &QPushButton::clicked, this, &CertCheckTab::acceptNew);
	connect(m_again, &QPushButton::clicked, this, &CertCheckTab::start);
	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	start();
}

QString CertCheckTab::edit(const QJsonObject& cmd) const
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

void CertCheckTab::start()
{
	m_table->setRowCount(0);
	m_accept->setEnabled(false);
	const QJsonObject r = parse(edit({ { QStringLiteral("op"), QStringLiteral("cert_check_start") }, { QStringLiteral("ids"), QJsonArray::fromStringList(m_ids) } }));
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(r.value(QStringLiteral("error")).toString());
		m_again->setEnabled(true);
		return;
	}
	m_again->setEnabled(false);
	const int total = r.value(QStringLiteral("count")).toInt();
	m_state->setText(m_backend->text(QStringLiteral("certcheck.busy")).arg(0).arg(total));
	if (!m_progress)
	{
		m_progress = new QTimer(this);
		m_progress->setInterval(500);
		connect(m_progress, &QTimer::timeout, this, [this] {
			const QJsonObject p = parse(edit({ { QStringLiteral("op"), QStringLiteral("cert_check_progress") } }));
			m_state->setText(m_backend->text(QStringLiteral("certcheck.busy")).arg(p.value(QStringLiteral("done")).toInt()).arg(p.value(QStringLiteral("total")).toInt()));
		});
	}
	m_progress->start();
}

void CertCheckTab::handleResult(const QJsonObject& r)
{
	if (m_progress)
		m_progress->stop();
	m_again->setEnabled(true);
	fill(r.value(QStringLiteral("rows")).toArray());
}

void CertCheckTab::fill(const QJsonArray& rows)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	m_table->setRowCount(rows.size());
	QMap<QString, int> count;
	for (int i = 0; i < rows.size(); ++i)
	{
		const QJsonObject o = rows.at(i).toObject();
		const QString state = o.value(QStringLiteral("state")).toString();
		const QString detail = o.value(QStringLiteral("detail")).toString();
		++count[state];
		QString text;
		if (state == QLatin1String("new"))
			text = T("certcheck.state.new");
		else if (state == QLatin1String("known"))
			text = T("certcheck.state.known");
		else if (state == QLatin1String("changed"))
			text = T("certcheck.state.changed").arg(detail);
		else if (state == QLatin1String("unreachable"))
			text = T("certcheck.state.unreachable").arg(detail);
		else if (state == QLatin1String("saved"))
			text = T("certcheck.state.saved");
		else if (state == QLatin1String("valid"))
			text = T("certcheck.state.valid");
		else
			text = T("certcheck.state.skipped").arg(detail);
		const QStringList cells = { o.value(QStringLiteral("name")).toString(), o.value(QStringLiteral("protocol")).toString().toUpper(),
			                        o.value(QStringLiteral("address")).toString(), o.value(QStringLiteral("kind")).toString(),
			                        o.value(QStringLiteral("fingerprint")).toString(), text };
		for (int c = 0; c < cells.size(); ++c)
		{
			auto* it = new QTableWidgetItem(cells.at(c));
			it->setToolTip(cells.at(c));
			if (state == QLatin1String("changed"))
			{
				it->setForeground(QColor(0xc0, 0x20, 0x20));
				QFont f = it->font();
				f.setBold(true);
				it->setFont(f);
			}
			m_table->setItem(i, c, it);
		}
	}
	const int fresh = count.value(QStringLiteral("new"));
	m_accept->setEnabled(fresh > 0);
	m_accept->setText(T("certcheck.accept.n").arg(fresh));
	m_state->setText(T("certcheck.summary")
	                   .arg(fresh)
	                   .arg(count.value(QStringLiteral("known")) + count.value(QStringLiteral("saved")) + count.value(QStringLiteral("valid")))
	                   .arg(count.value(QStringLiteral("changed")))
	                   .arg(count.value(QStringLiteral("unreachable")) + count.value(QStringLiteral("skipped"))));
}

void CertCheckTab::acceptNew()
{
	const QJsonObject r = parse(edit({ { QStringLiteral("op"), QStringLiteral("cert_check_accept") } }));
	fill(r.value(QStringLiteral("rows")).toArray());
	QString msg = m_backend->text(QStringLiteral("certcheck.saved")).arg(r.value(QStringLiteral("saved")).toInt());
	const QJsonArray errors = r.value(QStringLiteral("errors")).toArray();
	for (const QJsonValue& e : errors)
		msg += QStringLiteral("\n") + e.toString();
	m_state->setText(m_state->text() + QStringLiteral("\n") + msg);
	emit status(msg.section(QLatin1Char('\n'), 0, 0));
}

// ---------- UpdateTab ----------

UpdateTab::UpdateTab(RjBackend* backend, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	auto* lay = new QVBoxLayout(this);
	m_bar = new ActionBar(backend, this);
	lay->addWidget(m_bar);
	// Die Quelle ist fest (GitHub); einstellbar ist nur die Prüfung beim Start.
	m_onStart = new QCheckBox(T("update.on_start"), this);
	lay->addWidget(m_onStart);
	m_current = new QLabel(this);
	m_found = new QLabel(this);
	m_state = new QLabel(this);
	m_state->setWordWrap(true);
	m_state->setTextInteractionFlags(Qt::TextSelectableByMouse | Qt::TextSelectableByKeyboard);
	lay->addWidget(m_current);
	lay->addWidget(m_found);
	auto* notesLabel = new QLabel(T("update.notes"), this);
	m_notes = new QPlainTextEdit(this);
	m_notes->setReadOnly(true);
	notesLabel->setBuddy(m_notes);
	lay->addWidget(notesLabel);
	lay->addWidget(m_notes, 1);
	lay->addWidget(m_state);
	auto* row = new QHBoxLayout;
	m_check = new QPushButton(T("update.check"), this);
	m_install = new QPushButton(T("update.install"), this);
	m_later = new QPushButton(T("update.later"), this);
	m_cancel = new QPushButton(T("update.cancel"), this);
	m_restart = new QPushButton(T("update.restart"), this);
	m_install->setEnabled(false);
	m_cancel->hide();
	m_restart->hide();
	for (auto* b : { m_check, m_install, m_cancel, m_restart, m_later })
		row->addWidget(b);
	row->addStretch(1);
	lay->addLayout(row);
	connect(m_onStart, &QCheckBox::toggled, this, [this] { saveSettings(); });
	connect(m_check, &QPushButton::clicked, this, [this] { check(); });
	connect(m_install, &QPushButton::clicked, this, [this] { install(false); });
	connect(m_later, &QPushButton::clicked, this, [this] { emit closeRequested(); });
	connect(m_cancel, &QPushButton::clicked, this, [this] {
		m_backend->edit(QStringLiteral("{\"op\":\"update_cancel\"}"));
		setBusy(false, m_backend->text(QStringLiteral("update.cancelled")));
	});
	m_progressTimer = new QTimer(this);
	m_progressTimer->setInterval(300);
	connect(m_progressTimer, &QTimer::timeout, this, [this] { showProgress(); });
	connect(m_restart, &QPushButton::clicked, this, [this] {
		if (m_applyReady)
		{
			apply();
			return;
		}
		// Erst schließen (fragt bei ungespeicherten Änderungen nach); bei „Abbrechen“ bleibt alles, wie es ist.
		if (QWidget* w = window(); w && !w->close())
			return;
		// Neue Fassung starten; sie wartet, bis diese Instanz ihre Sperre freigibt.
		QProcess p;
		p.setProgram(m_program);
		auto env = QProcessEnvironment::systemEnvironment();
		env.insert(QStringLiteral("RJ_RESTART_WAIT"), QStringLiteral("1"));
		p.setProcessEnvironment(env);
		if (p.startDetached())
			qApp->quit();
		else
			m_state->setText(m_backend->text(QStringLiteral("update.restart.failed")));
	});
	refresh();
}

void UpdateTab::refresh()
{
	const QJsonObject s = QJsonDocument::fromJson(m_backend->edit(QStringLiteral("{\"op\":\"update_status\"}")).toUtf8()).object();
	const QSignalBlocker block(m_onStart);
	m_onStart->setChecked(s.value(QStringLiteral("check_on_start")).toBool());
	m_current->setText(m_backend->text(QStringLiteral("update.current")).arg(s.value(QStringLiteral("current")).toString(), s.value(QStringLiteral("platform")).toString()));
	m_pageOnly = s.value(QStringLiteral("page_only")).toBool();
	m_msi = s.value(QStringLiteral("msi")).toBool();
	if (m_pageOnly)
		m_install->setText(m_backend->text(QStringLiteral("update.open_page")));
	else if (m_msi)
		m_install->setText(m_backend->text(QStringLiteral("update.msi.download")));
	if (m_msi && !s.value(QStringLiteral("ready")).toString().isEmpty())
	{
		m_applyReady = true;
		m_restart->setText(m_backend->text(QStringLiteral("update.apply")));
		m_restart->show();
	}
	const QJsonObject f = s.value(QStringLiteral("found")).toObject();
	if (!f.isEmpty())
	{
		m_found->setText(m_backend->text(QStringLiteral("update.found")).arg(f.value(QStringLiteral("version")).toString()));
		m_notes->setPlainText(f.value(QStringLiteral("notes")).toString());
		m_install->setEnabled(true);
	}
	else
		m_found->setText(m_backend->text(QStringLiteral("update.found.none")));
	if (s.value(QStringLiteral("package_managed")).toBool())
		m_state->setText(m_backend->text(QStringLiteral("update.package_managed")));
	else if (s.value(QStringLiteral("development")).toBool())
		m_state->setText(m_backend->text(QStringLiteral("update.development")));
}

void UpdateTab::saveSettings()
{
	QJsonObject o;
	o.insert(QStringLiteral("op"), QStringLiteral("update_settings"));
	o.insert(QStringLiteral("check_on_start"), m_onStart->isChecked());
	const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Compact))).toUtf8()).object();
	if (r.contains(QStringLiteral("error")))
		m_state->setText(r.value(QStringLiteral("error")).toString());
}

void UpdateTab::setBusy(bool busy, const QString& text)
{
	m_check->setEnabled(!busy);
	m_install->setEnabled(!busy && m_install->isEnabled());
	m_cancel->setVisible(busy);
	if (!text.isEmpty())
		m_state->setText(text);
}

void UpdateTab::check()
{
	const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QStringLiteral("{\"op\":\"update_check\"}")).toUtf8()).object();
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(r.value(QStringLiteral("error")).toString());
		return;
	}
	setBusy(true, m_backend->text(QStringLiteral("update.checking")));
}

void UpdateTab::install(bool confirm)
{
	const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QStringLiteral("{\"op\":\"update_install\",\"confirm\":%1}").arg(confirm ? QStringLiteral("true") : QStringLiteral("false"))).toUtf8()).object();
	const QString state = r.value(QStringLiteral("state")).toString();
	if (r.contains(QStringLiteral("error")))
		m_state->setText(r.value(QStringLiteral("error")).toString());
	else if (state == QLatin1String("confirm"))
		m_bar->ask(m_backend->text(QStringLiteral("update.confirm")).arg(r.value(QStringLiteral("sessions")).toInt()), { m_backend->text(QStringLiteral("update.confirm.yes")) },
		           [this](int b) {
			           if (b == 0)
				           install(true);
		           });
	else if (state == QLatin1String("package"))
		m_state->setText(m_backend->text(QStringLiteral("update.package_managed.hint")).arg(r.value(QStringLiteral("url")).toString()));
	else if (state == QLatin1String("development"))
		m_state->setText(m_backend->text(QStringLiteral("update.development.hint")).arg(r.value(QStringLiteral("url")).toString()));
	else if (state == QLatin1String("page"))
	{
		// Windows: Release-Seite im Standardbrowser; die Adresse bleibt zum Kopieren stehen.
		const QString url = r.value(QStringLiteral("url")).toString();
		const bool opened = QDesktopServices::openUrl(QUrl(url));
		m_state->setText(m_backend->text(opened ? QStringLiteral("update.page.opened") : QStringLiteral("update.page.failed")).arg(url));
	}
	else if (state == QLatin1String("downloading"))
	{
		// Windows: MSI wird geladen und geprüft; Fortschritt aus dem Zustand.
		setBusy(true);
		showProgress();
		m_progressTimer->start();
	}
	else
		setBusy(true, m_backend->text(QStringLiteral("update.installing")));
}

void UpdateTab::showProgress()
{
	const QJsonObject s = QJsonDocument::fromJson(m_backend->edit(QStringLiteral("{\"op\":\"update_status\"}")).toUtf8()).object();
	const double mb = 1024.0 * 1024.0;
	const double got = s.value(QStringLiteral("progress")).toDouble() / mb, total = s.value(QStringLiteral("total")).toDouble() / mb;
	m_state->setText(m_backend->text(QStringLiteral("update.downloading")).arg(QString::number(got, 'f', 1), QString::number(total, 'f', 1)));
}

void UpdateTab::apply()
{
	// Offene Sitzungen werden mit dem Beenden getrennt: vorher fragen.
	const QJsonObject s = QJsonDocument::fromJson(m_backend->edit(QStringLiteral("{\"op\":\"update_status\"}")).toUtf8()).object();
	const int open = s.value(QStringLiteral("sessions")).toInt();
	auto go = [this] {
		// Erst schließen (fragt bei ungespeicherten Änderungen nach), dann den Hilfsprozess starten: Er installiert,
		// sobald remotejoe beendet ist. Bei „Abbrechen“ bleibt alles, wie es ist.
		QWidget* w = window();
		if (w && !w->close())
			return;
		const QJsonObject r = QJsonDocument::fromJson(m_backend->edit(QStringLiteral("{\"op\":\"update_apply\",\"confirm\":true}")).toUtf8()).object();
		if (r.value(QStringLiteral("state")).toString() == QLatin1String("applying"))
		{
			qApp->quit();
			return;
		}
		// Nicht startbar: Fenster wieder zeigen, Grund nennen.
		if (w)
			w->show();
		m_state->setText(m_backend->text(QStringLiteral("update.error")).arg(r.value(QStringLiteral("error")).toString()));
	};
	if (open > 0)
		m_bar->ask(m_backend->text(QStringLiteral("update.apply.confirm")).arg(open), { m_backend->text(QStringLiteral("update.apply")) }, [go](int b) {
			if (b == 0)
				go();
		});
	else
		go();
}

void UpdateTab::handleResult(const QString& kind, const QJsonObject& r)
{
	setBusy(false);
	m_progressTimer->stop();
	const QString state = r.value(QStringLiteral("state")).toString();
	if (state == QLatin1String("error"))
	{
		m_state->setText(m_backend->text(QStringLiteral("update.error")).arg(r.value(QStringLiteral("error")).toString()));
		return;
	}
	if (kind == QLatin1String("update_check"))
	{
		if (state == QLatin1String("newer"))
		{
			const QJsonObject f = r.value(QStringLiteral("found")).toObject();
			m_found->setText(m_backend->text(QStringLiteral("update.found")).arg(f.value(QStringLiteral("version")).toString()));
			m_notes->setPlainText(f.value(QStringLiteral("notes")).toString());
			m_install->setEnabled(true);
			// Windows: Veröffentlichung ohne MSI → Release-Seite wie ohne MSI-Installation.
			if (r.contains(QStringLiteral("no_msi")))
			{
				m_pageOnly = true;
				m_install->setText(m_backend->text(QStringLiteral("update.open_page")));
			}
			m_state->setText(m_backend->text(QStringLiteral("update.newer"))
			                 + (m_pageOnly ? QLatin1Char(' ') + m_backend->text(QStringLiteral("update.windows.hint")) : QString()));
			m_install->setFocus();
		}
		else
		{
			m_found->setText(m_backend->text(QStringLiteral("update.found.none")));
			m_notes->clear();
			m_install->setEnabled(false);
			m_state->setText(m_backend->text(QStringLiteral("update.uptodate")).arg(r.value(QStringLiteral("latest")).toString()));
		}
	}
	else if (state == QLatin1String("ready"))
	{
		// Windows: MSI geladen, Prüfsumme und Signaturen geprüft.
		m_applyReady = true;
		m_install->setEnabled(false);
		m_state->setText(m_backend->text(QStringLiteral("update.ready")).arg(r.value(QStringLiteral("version")).toString()));
		m_restart->setText(m_backend->text(QStringLiteral("update.apply")));
		m_restart->show();
		m_restart->setFocus();
	}
	else if (state == QLatin1String("installed"))
	{
		m_program = r.value(QStringLiteral("program")).toString();
		m_install->setEnabled(false);
		m_state->setText(m_backend->text(QStringLiteral("update.installed")).arg(r.value(QStringLiteral("version")).toString(), r.value(QStringLiteral("old")).toString()));
		m_restart->show();
		m_restart->setFocus();
	}
}
