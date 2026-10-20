#include "overviewtab.h"

#include "cards.h"
#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonDocument>
#include <QtCore/QTimer>
#include <QtGui/QFontDatabase>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QGridLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QScrollArea>
#include <QtWidgets/QScrollBar>
#include <QtWidgets/QVBoxLayout>

namespace
{
QString when(qint64 secs)
{
	return secs > 0 ? QDateTime::fromSecsSinceEpoch(secs).toString(QStringLiteral("dd.MM.yyyy HH:mm")) : QString();
}

QLabel* muted(const QString& text, QWidget* parent)
{
	auto* l = new QLabel(text, parent);
	l->setObjectName(QStringLiteral("rjMuted"));
	return l;
}

/// Felder in den Kacheln (Name und System stehen in der Kopfkarte).
const char* const kTiles[4] = { "vcpu", "memory", "ip", "mac" };
} // namespace

OverviewTab::OverviewTab(RjBackend* backend, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setObjectName(QStringLiteral("overviewTab"));
	auto* outer = new QVBoxLayout(this);
	outer->setContentsMargins(0, 0, 0, 0);
	m_scroll = new QScrollArea(this);
	m_scroll->setWidgetResizable(true);
	m_scroll->setFrameShape(QFrame::NoFrame);
	outer->addWidget(m_scroll);
	auto* page = new QWidget(m_scroll);
	m_scroll->setWidget(page);
	auto* lay = new QVBoxLayout(page);
	lay->setContentsMargins(12, 8, 12, 12);
	lay->setSpacing(10);

	m_crumb = muted(QString(), page);
	lay->addWidget(m_crumb);

	// Kopfkarte
	QVBoxLayout* in = nullptr;
	auto* head = rjui::card(page, &in);
	auto* hh = new QHBoxLayout;
	hh->setSpacing(12);
	m_icon = new QLabel(head);
	m_icon->setFixedSize(36, 36);
	hh->addWidget(m_icon, 0, Qt::AlignTop);
	auto* hv = new QVBoxLayout;
	hv->setSpacing(2);
	m_title = new QLabel(head);
	QFont tf = m_title->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.45);
	tf.setBold(true);
	m_title->setFont(tf);
	m_title->setTextInteractionFlags(Qt::TextSelectableByMouse);
	m_sub = muted(QString(), head);
	m_sub->setWordWrap(true);
	m_sub->setTextInteractionFlags(Qt::TextSelectableByMouse);
	hv->addWidget(m_title);
	hv->addWidget(m_sub);
	hh->addLayout(hv, 1);
	m_connect = new QPushButton(T("ov.connect"), head);
	hh->addWidget(m_connect, 0, Qt::AlignTop);
	in->addLayout(hh);
	lay->addWidget(head);
	connect(m_connect, &QPushButton::clicked, this, [this] { emit connectRequested(m_subject); });

	// Kennzahlen
	auto* inv = rjui::card(page, &in);
	auto* ih = new QHBoxLayout;
	ih->addWidget(rjui::section(T("ov.inventory"), inv));
	ih->addStretch(1);
	m_editToggle = new QPushButton(T("ov.inventory.edit"), inv);
	m_editToggle->setCheckable(true);
	m_query = new QPushButton(T("ov.inventory.query"), inv);
	m_query->setToolTip(T("ov.inventory.query.tip"));
	// Schalter: darf sich das Inventar am Ziel anmelden (SSH/PowerShell)? Bei Web-Zielen ohne Festlegung aus.
	m_inventoryOn = new QCheckBox(T("ov.inventory.allowed"), inv);
	m_inventoryOn->setToolTip(T("ov.inventory.allowed.tip"));
	ih->addWidget(m_inventoryOn);
	ih->addWidget(m_editToggle);
	ih->addWidget(m_query);
	in->addLayout(ih);
	m_inventoryRule = muted(QString(), inv);
	in->addWidget(m_inventoryRule);
	auto* tiles = new QGridLayout;
	tiles->setSpacing(8);
	for (int i = 0; i < 4; ++i)
	{
		tiles->addWidget(rjui::tile(fieldLabel(QString::fromLatin1(kTiles[i])), &m_tileValue[i], inv), 0, i);
		tiles->setColumnStretch(i, 1);
	}
	in->addLayout(tiles);
	m_inventorySource = muted(QString(), inv);
	in->addWidget(m_inventorySource);
	m_pendingBox = new QWidget(inv);
	m_pending = new QVBoxLayout(m_pendingBox);
	m_pending->setContentsMargins(0, 0, 0, 0);
	in->addWidget(m_pendingBox);
	m_inventoryBox = new QWidget(inv);
	m_inventoryGrid = new QGridLayout(m_inventoryBox);
	m_inventoryGrid->setContentsMargins(0, 4, 0, 0);
	m_inventoryGrid->setColumnStretch(1, 1);
	m_inventoryBox->hide();
	in->addWidget(m_inventoryBox);
	m_queryState = new QLabel(inv);
	m_queryState->setWordWrap(true);
	m_queryState->setTextFormat(Qt::RichText);
	m_queryState->setOpenExternalLinks(true);
	m_queryState->hide();
	in->addWidget(m_queryState);
	lay->addWidget(inv);
	connect(m_query, &QPushButton::clicked, this, [this] { query(); });
	connect(m_inventoryOn, &QCheckBox::toggled, this, [this](bool on) {
		const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("ov_inventory") }, { QStringLiteral("on"), on } });
		if (r.contains(QStringLiteral("error")))
		{
			setQueryState(r.value(QStringLiteral("error")).toString());
			return;
		}
		applyInventorySwitch(r);
	});
	connect(m_editToggle, &QPushButton::toggled, m_inventoryBox, &QWidget::setVisible);

	// Dienste
	auto* svc = rjui::card(page, &in);
	auto* sh = new QHBoxLayout;
	sh->addWidget(rjui::section(T("ov.services"), svc));
	sh->addStretch(1);
	m_servicesInfo = muted(QString(), svc);
	sh->addWidget(m_servicesInfo);
	m_servicesAgain = new QPushButton(T("ov.services.again"), svc);
	sh->addWidget(m_servicesAgain);
	in->addLayout(sh);
	m_chipsBox = new rjui::FlowBox(svc, 6);
	m_chips = m_chipsBox->flow();
	in->addWidget(m_chipsBox);
	lay->addWidget(svc);
	connect(m_servicesAgain, &QPushButton::clicked, this, [this] { probe(true); });

	// Notizen
	m_notesCard = rjui::card(page, &in);
	in->addWidget(rjui::section(T("ov.notes"), m_notesCard));
	m_notes = new QPlainTextEdit(m_notesCard);
	m_notes->setTabChangesFocus(true);
	m_notes->setMinimumHeight(60);
	m_notes->setMaximumHeight(120);
	m_notes->setPlaceholderText(T("ov.notes.hint"));
	in->addWidget(m_notes);
	auto* nh = new QHBoxLayout;
	m_notesState = muted(QString(), m_notesCard);
	nh->addWidget(m_notesState, 1);
	m_notesSave = new QPushButton(T("ov.notes.save"), m_notesCard);
	m_notesSave->setEnabled(false);
	nh->addWidget(m_notesSave);
	in->addLayout(nh);
	lay->addWidget(m_notesCard);
	connect(m_notes, &QPlainTextEdit::textChanged, this, [this] {
		m_notesSave->setEnabled(m_notesEditable);
		m_notesState->clear();
	});
	connect(m_notesSave, &QPushButton::clicked, this, [this] { saveNotes(); });

	// Prüfen
	auto* chk = rjui::card(page, &in);
	auto* ch = new QHBoxLayout;
	ch->addWidget(rjui::section(T("ov.checks"), chk));
	ch->addStretch(1);
	m_ping = new QPushButton(T("ov.ping"), chk);
	m_trace = new QPushButton(T("ov.trace"), chk);
	m_stop = new QPushButton(T("ov.stop"), chk);
	m_stop->setEnabled(false);
	ch->addWidget(m_ping);
	ch->addWidget(m_trace);
	ch->addWidget(m_stop);
	in->addLayout(ch);
	m_output = new QPlainTextEdit(chk);
	m_output->setReadOnly(true);
	m_output->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
	m_output->setMinimumHeight(160);
	m_output->hide();
	in->addWidget(m_output);
	lay->addWidget(chk);
	connect(m_ping, &QPushButton::clicked, this, [this] { startCheck(QStringLiteral("ping")); });
	connect(m_trace, &QPushButton::clicked, this, [this] { startCheck(QStringLiteral("trace")); });
	connect(m_stop, &QPushButton::clicked, this, [this] {
		call({ { QStringLiteral("op"), QStringLiteral("ov_stop") } });
		pollRun();
	});

	// Aruba (Schritt 2: AP-Check)
	m_arubaCard = rjui::card(page, &in);
	in->addWidget(rjui::section(T("ov.aruba"), m_arubaCard));
	auto* al = new QLabel(T("ov.aruba.later"), m_arubaCard);
	al->setWordWrap(true);
	in->addWidget(al);
	m_arubaCard->hide();
	lay->addWidget(m_arubaCard);
	lay->addStretch(1);

	m_poll = new QTimer(this);
	m_poll->setInterval(300);
	connect(m_poll, &QTimer::timeout, this, [this] { pollRun(); });
}

QJsonObject OverviewTab::call(const QJsonObject& c) const
{
	QJsonObject cmd = m_subject;
	for (auto it = c.begin(); it != c.end(); ++it)
		cmd.insert(it.key(), it.value());
	return QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
}

void OverviewTab::showSubject(const QJsonObject& subject, const QIcon& icon)
{
	if (subject == m_subject)
		return;
	if (m_notesSave->isEnabled() && !m_subject.isEmpty())
		saveNotes(); // Ungespeicherte Notiz des vorherigen Ziels nicht verlieren.
	m_subject = subject;
	m_icon->setPixmap(icon.isNull() ? QPixmap() : icon.pixmap(32, 32));
	m_output->clear();
	m_output->hide();
	m_editToggle->setChecked(false);
	m_scroll->verticalScrollBar()->setValue(0);
	load();
	probe(false);
}

void OverviewTab::applyInventorySwitch(const QJsonObject& r)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const bool guest = r.value(QStringLiteral("guest")).toBool();
	const bool on = r.value(QStringLiteral("can_query")).toBool();
	// Knopf bleibt sichtbar (kein Springen beim Umschalten), ist aber nur bei erlaubtem Inventar bedienbar.
	m_query->setVisible(!guest);
	m_query->setEnabled(on && !r.value(QStringLiteral("busy")).toBool());
	const QSignalBlocker block(m_inventoryOn);
	m_inventoryOn->setVisible(!guest);
	m_inventoryOn->setChecked(on);
	m_inventoryOn->setEnabled(r.value(QStringLiteral("inventory_switchable")).toBool());
	m_inventoryRule->setVisible(!guest);
	m_inventoryRule->setText(T("ov.inventory.rule").arg(on ? T("ov.inventory.rule.on") : T("ov.inventory.rule.off"), r.value(QStringLiteral("inventory_source")).toString()));
}

QString OverviewTab::fieldLabel(const QString& key) const
{
	return m_backend->text(QStringLiteral("ov.field.") + key);
}

void OverviewTab::load()
{
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("overview") } });
	if (r.contains(QStringLiteral("error")))
	{
		m_title->setText(r.value(QStringLiteral("error")).toString());
		return;
	}
	m_key = r.value(QStringLiteral("key")).toString();
	m_name = r.value(QStringLiteral("name")).toString();
	m_title->setText(m_name);
	m_subBase = r.value(QStringLiteral("sub")).toString();
	m_crumb->setText(r.value(QStringLiteral("path")).toString());
	m_crumb->setVisible(!m_crumb->text().isEmpty());
	const bool guest = r.value(QStringLiteral("guest")).toBool();
	applyInventorySwitch(r);
	m_notesCard->setVisible(!guest);
	emit titleChanged();
	renderServices(r.value(QStringLiteral("services")));
	renderInventory(r.value(QStringLiteral("inventory")).toArray());
	setQueryState(QString());
	m_notesEditable = r.value(QStringLiteral("notes_editable")).toBool();
	const QSignalBlocker block(m_notes);
	m_notes->setPlainText(r.value(QStringLiteral("notes")).toString());
	m_notes->setReadOnly(!m_notesEditable);
	m_notesSave->setEnabled(false);
	m_notesState->clear();
	m_arubaCard->setVisible(r.value(QStringLiteral("aruba")).toBool());
	if (r.value(QStringLiteral("run")).isObject())
	{
		m_output->show();
		pollRun();
	}
}

void OverviewTab::probe(bool force)
{
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("ov_probe") }, { QStringLiteral("force"), force } });
	if (r.contains(QStringLiteral("pending")))
	{
		m_servicesInfo->setText(m_backend->text(QStringLiteral("ov.services.running")));
		m_servicesAgain->setEnabled(false);
	}
	else if (r.contains(QStringLiteral("services")))
		renderServices(r.value(QStringLiteral("services")));
	else if (r.contains(QStringLiteral("error")))
		m_servicesInfo->setText(r.value(QStringLiteral("error")).toString());
}

void OverviewTab::query()
{
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("ov_query") } });
	if (r.contains(QStringLiteral("error")))
	{
		showQueryError(r);
		return;
	}
	m_query->setEnabled(false);
	setQueryState(m_backend->text(QStringLiteral("ov.inventory.running")).toHtmlEscaped());
}

void OverviewTab::handleResult(const QString& kind, const QString& target, const QJsonObject& r)
{
	if (target != m_key)
		return;
	if (kind == QLatin1String("ov_probe"))
	{
		m_servicesAgain->setEnabled(true);
		renderServices(r.value(QStringLiteral("services")));
		if (r.value(QStringLiteral("inventory")).isArray())
			renderInventory(r.value(QStringLiteral("inventory")).toArray());
	}
	else if (kind == QLatin1String("ov_query"))
	{
		m_query->setEnabled(m_inventoryOn->isChecked());
		if (r.contains(QStringLiteral("inventory")))
		{
			renderInventory(r.value(QStringLiteral("inventory")).toArray());
			QString done = m_backend->text(QStringLiteral("ov.inventory.done")).toHtmlEscaped();
			if (r.value(QStringLiteral("note")).isString())
				done += QStringLiteral("<br><b>%1</b>").arg(r.value(QStringLiteral("note")).toString().toHtmlEscaped());
			setQueryState(done);
			// Aruba erst nach der Abfrage bekannt.
			m_arubaCard->setVisible(call({ { QStringLiteral("op"), QStringLiteral("overview") } }).value(QStringLiteral("aruba")).toBool());
		}
		if (r.value(QStringLiteral("error")).isString())
			showQueryError(r);
	}
}

void OverviewTab::renderServices(const QJsonValue& s)
{
	while (QLayoutItem* it = m_chips->takeAt(0))
	{
		if (QWidget* w = it->widget())
			w->deleteLater();
		delete it;
	}
	if (!s.isObject())
	{
		m_servicesInfo->setText(m_backend->text(QStringLiteral("ov.services.running")));
		return;
	}
	const QJsonObject o = s.toObject();
	if (o.contains(QStringLiteral("error")))
	{
		m_servicesInfo->setText(o.value(QStringLiteral("error")).toString());
		return;
	}
	for (const QJsonValue& v : o.value(QStringLiteral("list")).toArray())
	{
		const QJsonObject e = v.toObject();
		const bool open = e.value(QStringLiteral("open")).toBool();
		const QString label = QStringLiteral("%1 %2").arg(e.value(QStringLiteral("name")).toString()).arg(e.value(QStringLiteral("port")).toInt());
		auto* chip = rjui::pill(open ? QStringLiteral("● ") + label : label, open ? QStringLiteral("ok") : QStringLiteral("off"), m_chipsBox);
		chip->setToolTip(m_backend->text(open ? QStringLiteral("ov.service.open") : QStringLiteral("ov.service.closed")));
		m_chips->addWidget(chip);
	}
	m_servicesInfo->setText(QStringLiteral("%1 · %2").arg(o.value(QStringLiteral("ip")).toString(), when(o.value(QStringLiteral("at")).toInteger())));
	m_chipsBox->relayout();
}

void OverviewTab::updateSub()
{
	// Kurzzeile: Protokoll und Adresse, dazu System und Rechnername aus dem Inventar.
	QStringList parts{ m_subBase };
	QString name, system;
	for (const QJsonValue& v : m_inventory)
	{
		const QJsonObject f = v.toObject();
		if (f.value(QStringLiteral("key")).toString() == QLatin1String("system"))
			system = f.value(QStringLiteral("value")).toString();
		else if (f.value(QStringLiteral("key")).toString() == QLatin1String("name"))
			name = f.value(QStringLiteral("value")).toString();
	}
	if (!system.isEmpty())
		parts << system;
	if (!name.isEmpty() && name.compare(m_name, Qt::CaseInsensitive) != 0)
		parts << m_backend->text(QStringLiteral("ov.machine")).arg(name);
	m_sub->setText(parts.join(QStringLiteral(" · ")));
}

void OverviewTab::renderInventory(const QJsonArray& fields)
{
	m_inventory = fields;
	updateSub();
	// Kacheln und Quelle (jüngster Eintrag)
	qint64 latest = 0;
	QString latestSource;
	for (const QJsonValue& v : fields)
	{
		const QJsonObject f = v.toObject();
		const QString key = f.value(QStringLiteral("key")).toString();
		for (int i = 0; i < 4; ++i)
			if (key == QLatin1String(kTiles[i]))
			{
				const QString value = f.value(QStringLiteral("value")).toString();
				m_tileValue[i]->setText(value.isEmpty() ? QStringLiteral("–") : value);
				m_tileValue[i]->setToolTip(f.value(QStringLiteral("source")).toString());
			}
		const qint64 at = f.value(QStringLiteral("at")).toInteger();
		if (at > latest && !f.value(QStringLiteral("manual")).toBool())
		{
			latest = at;
			latestSource = f.value(QStringLiteral("source")).toString();
		}
	}
	m_inventorySource->setText(latest > 0 ? m_backend->text(QStringLiteral("ov.inventory.source")).arg(latestSource, when(latest))
	                                      : m_backend->text(QStringLiteral("ov.inventory.empty")));

	// Vorgemerkte neue Werte
	while (QLayoutItem* it = m_pending->takeAt(0))
	{
		if (QWidget* w = it->widget())
			w->deleteLater();
		delete it;
	}
	// Bearbeiten: alle Felder
	while (QLayoutItem* it = m_inventoryGrid->takeAt(0))
	{
		if (QWidget* w = it->widget())
			w->deleteLater();
		delete it;
	}
	int row = 0, pendingCount = 0;
	for (const QJsonValue& v : fields)
	{
		const QJsonObject f = v.toObject();
		const QString key = f.value(QStringLiteral("key")).toString();
		const QJsonObject p = f.value(QStringLiteral("pending")).toObject();
		if (!p.isEmpty())
		{
			auto* box = new QWidget(m_pendingBox);
			auto* h = new QHBoxLayout(box);
			h->setContentsMargins(0, 0, 0, 0);
			h->addWidget(rjui::pill(fieldLabel(key), QStringLiteral("warn"), box));
			auto* note = new QLabel(m_backend->text(QStringLiteral("ov.pending")).arg(p.value(QStringLiteral("value")).toString(), p.value(QStringLiteral("source")).toString(), when(p.value(QStringLiteral("at")).toInteger())), box);
			note->setWordWrap(true);
			auto* take = new QPushButton(m_backend->text(QStringLiteral("ov.pending.accept")), box);
			auto* keep = new QPushButton(m_backend->text(QStringLiteral("ov.pending.reject")), box);
			h->addWidget(note, 1);
			h->addWidget(take);
			h->addWidget(keep);
			for (auto [b, accept] : { std::pair{ take, true }, std::pair{ keep, false } })
				connect(b, &QPushButton::clicked, this, [this, key, accept] {
					const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("ov_field") }, { QStringLiteral("field"), key }, { QStringLiteral("accept"), accept } });
					QTimer::singleShot(0, this, [this, inv = r.value(QStringLiteral("inventory")).toArray()] { renderInventory(inv); });
				});
			m_pending->addWidget(box);
			++pendingCount;
		}
		auto* label = new QLabel(fieldLabel(key), m_inventoryBox);
		auto* edit = new QLineEdit(f.value(QStringLiteral("value")).toString(), m_inventoryBox);
		edit->setPlaceholderText(m_backend->text(QStringLiteral("ov.field.unknown")));
		label->setBuddy(edit);
		QString src = f.value(QStringLiteral("source")).toString();
		if (const QString w = when(f.value(QStringLiteral("at")).toInteger()); !w.isEmpty() && !src.isEmpty())
			src += QStringLiteral(", ") + w;
		auto* source = muted(src, m_inventoryBox);
		// Feste Breite: Die Eingabefelder springen beim Zielwechsel nicht mit der Länge der Quellenangabe.
		source->setFixedWidth(source->fontMetrics().horizontalAdvance(QStringLiteral("SSH (show version), 00.00.0000 00:00")) + 8);
		source->setToolTip(src);
		m_inventoryGrid->addWidget(label, row, 0);
		m_inventoryGrid->addWidget(edit, row, 1);
		m_inventoryGrid->addWidget(source, row, 2);
		const QString before = edit->text();
		connect(edit, &QLineEdit::editingFinished, this, [this, edit, key, before] {
			if (edit->text().trimmed() == before)
				return;
			const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("ov_field") }, { QStringLiteral("field"), key }, { QStringLiteral("value"), edit->text() } });
			if (r.contains(QStringLiteral("error")))
				setQueryState(r.value(QStringLiteral("error")).toString().toHtmlEscaped());
			else
				QTimer::singleShot(0, this, [this, inv = r.value(QStringLiteral("inventory")).toArray()] { renderInventory(inv); });
		});
		++row;
	}
	m_pendingBox->setVisible(pendingCount > 0);
}

void OverviewTab::startCheck(const QString& kind)
{
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("ov_run") }, { QStringLiteral("kind"), kind } });
	m_output->show();
	if (r.contains(QStringLiteral("error")))
	{
		m_output->setPlainText(r.value(QStringLiteral("error")).toString());
		return;
	}
	m_output->clear();
	m_ping->setEnabled(false);
	m_trace->setEnabled(false);
	m_stop->setEnabled(true);
	m_poll->start();
}

void OverviewTab::pollRun()
{
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("ov_output") } }).value(QStringLiteral("run")).toObject();
	const bool done = r.isEmpty() || r.value(QStringLiteral("done")).toBool();
	if (!r.isEmpty())
	{
		const QString text = r.value(QStringLiteral("text")).toString();
		if (text != m_output->toPlainText())
		{
			m_output->setPlainText(text);
			m_output->moveCursor(QTextCursor::End);
		}
	}
	m_ping->setEnabled(done);
	m_trace->setEnabled(done);
	m_stop->setEnabled(!done);
	if (done)
		m_poll->stop();
	else if (!m_poll->isActive())
		m_poll->start();
}

void OverviewTab::saveNotes()
{
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("ov_notes") }, { QStringLiteral("notes"), m_notes->toPlainText() } });
	if (r.contains(QStringLiteral("error")))
	{
		m_notesState->setText(r.value(QStringLiteral("error")).toString());
		return;
	}
	m_notesSave->setEnabled(false);
	m_notesState->setText(m_backend->text(QStringLiteral("ov.notes.saved")));
	emit indexChanged();
}

void OverviewTab::showQueryError(const QJsonObject& r)
{
	// Fehlermeldung mit Verweisen auf die passende Anleitung (WinRM bzw. OpenSSH unter Windows).
	QString html = r.value(QStringLiteral("error")).toString().toHtmlEscaped();
	for (const QJsonValue& h : r.value(QStringLiteral("help")).toArray())
		html += QStringLiteral("<br><a href=\"%1\">%2</a>").arg(h.toObject().value(QStringLiteral("url")).toString().toHtmlEscaped(), h.toObject().value(QStringLiteral("text")).toString().toHtmlEscaped());
	setQueryState(html);
}

void OverviewTab::setQueryState(const QString& html)
{
	m_queryState->setText(html);
	m_queryState->setVisible(!html.isEmpty());
}
