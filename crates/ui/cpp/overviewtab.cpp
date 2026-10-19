#include "overviewtab.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QJsonDocument>
#include <QtCore/QTimer>
#include <QtGui/QFontDatabase>
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
QLabel* heading(const QString& text, QWidget* parent)
{
	auto* l = new QLabel(text, parent);
	l->setObjectName(QStringLiteral("heading"));
	return l;
}

QString when(qint64 secs)
{
	return secs > 0 ? QDateTime::fromSecsSinceEpoch(secs).toString(QStringLiteral("dd.MM.yyyy HH:mm")) : QString();
}
} // namespace

OverviewTab::OverviewTab(RjBackend* backend, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setObjectName(QStringLiteral("overviewTab"));
	auto* outer = new QVBoxLayout(this);
	outer->setContentsMargins(0, 0, 0, 0);
	auto* scroll = m_scroll = new QScrollArea(this);
	scroll->setWidgetResizable(true);
	scroll->setFrameShape(QFrame::NoFrame);
	outer->addWidget(scroll);
	auto* page = new QWidget(scroll);
	scroll->setWidget(page);
	auto* lay = new QVBoxLayout(page);

	// Zielname
	m_title = new QLabel(page);
	QFont tf = m_title->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.6);
	tf.setBold(true);
	m_title->setFont(tf);
	m_title->setTextInteractionFlags(Qt::TextSelectableByMouse);
	lay->addWidget(m_title);
	m_sub = new QLabel(page);
	m_sub->setTextInteractionFlags(Qt::TextSelectableByMouse);
	lay->addWidget(m_sub);

	// Angebotene Dienste
	auto* sh = new QHBoxLayout;
	sh->addWidget(heading(T("ov.services"), page));
	sh->addStretch(1);
	m_servicesAgain = new QPushButton(T("ov.services.again"), page);
	sh->addWidget(m_servicesAgain);
	lay->addLayout(sh);
	m_services = new QLabel(page);
	m_services->setWordWrap(true);
	m_services->setTextFormat(Qt::RichText);
	lay->addWidget(m_services);
	connect(m_servicesAgain, &QPushButton::clicked, this, [this] { probe(true); });

	// Inventar
	auto* ih = new QHBoxLayout;
	ih->addWidget(heading(T("ov.inventory"), page));
	ih->addStretch(1);
	m_query = new QPushButton(T("ov.inventory.query"), page);
	m_query->setToolTip(T("ov.inventory.query.tip"));
	ih->addWidget(m_query);
	lay->addLayout(ih);
	m_inventoryBox = new QWidget(page);
	m_inventory = new QGridLayout(m_inventoryBox);
	m_inventory->setContentsMargins(0, 0, 0, 0);
	m_inventory->setColumnStretch(1, 1);
	lay->addWidget(m_inventoryBox);
	m_queryState = new QLabel(page);
	m_queryState->setWordWrap(true);
	m_queryState->setTextFormat(Qt::RichText);
	m_queryState->setOpenExternalLinks(true);
	lay->addWidget(m_queryState);
	connect(m_query, &QPushButton::clicked, this, [this] { query(); });

	// Notizen
	m_notesHead = heading(T("ov.notes"), page);
	lay->addWidget(m_notesHead);
	m_notes = new QPlainTextEdit(page);
	m_notes->setTabChangesFocus(true);
	m_notes->setMinimumHeight(70);
	m_notes->setMaximumHeight(140);
	m_notes->setPlaceholderText(T("ov.notes.hint"));
	lay->addWidget(m_notes);
	auto* nh = new QHBoxLayout;
	m_notesState = new QLabel(page);
	nh->addWidget(m_notesState, 1);
	m_notesSave = new QPushButton(T("ov.notes.save"), page);
	m_notesSave->setEnabled(false);
	nh->addWidget(m_notesSave);
	lay->addLayout(nh);
	connect(m_notes, &QPlainTextEdit::textChanged, this, [this] {
		m_notesSave->setEnabled(m_notesEditable);
		m_notesState->clear();
	});
	connect(m_notesSave, &QPushButton::clicked, this, [this] { saveNotes(); });

	// Prüfungen
	lay->addWidget(heading(T("ov.checks"), page));
	auto* ch = new QHBoxLayout;
	m_ping = new QPushButton(T("ov.ping"), page);
	m_trace = new QPushButton(T("ov.trace"), page);
	m_stop = new QPushButton(T("ov.stop"), page);
	m_stop->setEnabled(false);
	ch->addWidget(m_ping);
	ch->addWidget(m_trace);
	ch->addWidget(m_stop);
	ch->addStretch(1);
	lay->addLayout(ch);
	m_output = new QPlainTextEdit(page);
	m_output->setReadOnly(true);
	m_output->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
	m_output->setMinimumHeight(180);
	m_output->hide();
	lay->addWidget(m_output);
	connect(m_ping, &QPushButton::clicked, this, [this] { startCheck(QStringLiteral("ping")); });
	connect(m_trace, &QPushButton::clicked, this, [this] { startCheck(QStringLiteral("trace")); });
	connect(m_stop, &QPushButton::clicked, this, [this] {
		call({ { QStringLiteral("op"), QStringLiteral("ov_stop") } });
		pollRun();
	});

	// Aruba (Schritt 2: AP-Check)
	m_arubaHead = heading(T("ov.aruba"), page);
	m_aruba = new QLabel(T("ov.aruba.later"), page);
	m_aruba->setWordWrap(true);
	lay->addWidget(m_arubaHead);
	lay->addWidget(m_aruba);
	m_arubaHead->hide();
	m_aruba->hide();
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

void OverviewTab::showSubject(const QJsonObject& subject)
{
	if (subject == m_subject)
		return;
	if (m_notesSave->isEnabled() && !m_subject.isEmpty())
		saveNotes(); // Ungespeicherte Notiz des vorherigen Ziels nicht verlieren.
	m_subject = subject;
	m_output->clear();
	m_output->hide();
	m_scroll->verticalScrollBar()->setValue(0);
	load();
	probe(false);
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
	m_sub->setText(r.value(QStringLiteral("sub")).toString());
	const bool guest = r.value(QStringLiteral("guest")).toBool();
	m_query->setVisible(r.value(QStringLiteral("can_query")).toBool());
	for (QWidget* w : { static_cast<QWidget*>(m_notesHead), static_cast<QWidget*>(m_notes), static_cast<QWidget*>(m_notesSave), static_cast<QWidget*>(m_notesState) })
		w->setVisible(!guest);
	emit titleChanged();
	renderServices(r.value(QStringLiteral("services")));
	renderInventory(r.value(QStringLiteral("inventory")).toArray());
	const bool busy = r.value(QStringLiteral("busy")).toBool();
	m_query->setEnabled(!busy);
	m_queryState->clear();
	m_notesEditable = r.value(QStringLiteral("notes_editable")).toBool();
	const QSignalBlocker block(m_notes);
	m_notes->setPlainText(r.value(QStringLiteral("notes")).toString());
	m_notes->setReadOnly(!m_notesEditable);
	m_notesSave->setEnabled(false);
	m_notesState->clear();
	const bool aruba = r.value(QStringLiteral("aruba")).toBool();
	m_arubaHead->setVisible(aruba);
	m_aruba->setVisible(aruba);
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
		m_services->setText(m_backend->text(QStringLiteral("ov.services.running")));
		m_servicesAgain->setEnabled(false);
	}
	else if (r.contains(QStringLiteral("services")))
		renderServices(r.value(QStringLiteral("services")));
	else if (r.contains(QStringLiteral("error")))
		m_services->setText(r.value(QStringLiteral("error")).toString().toHtmlEscaped());
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
	m_queryState->setText(m_backend->text(QStringLiteral("ov.inventory.running")));
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
		m_query->setEnabled(true);
		if (r.contains(QStringLiteral("inventory")))
		{
			renderInventory(r.value(QStringLiteral("inventory")).toArray());
			m_queryState->setText(m_backend->text(QStringLiteral("ov.inventory.done")));
			// Aruba erst nach der Abfrage bekannt.
			const QJsonObject o = call({ { QStringLiteral("op"), QStringLiteral("overview") } });
			const bool aruba = o.value(QStringLiteral("aruba")).toBool();
			m_arubaHead->setVisible(aruba);
			m_aruba->setVisible(aruba);
		}
		if (r.value(QStringLiteral("error")).isString())
			showQueryError(r);
	}
}

void OverviewTab::renderServices(const QJsonValue& s)
{
	if (!s.isObject())
	{
		m_services->setText(m_backend->text(QStringLiteral("ov.services.running")));
		return;
	}
	const QJsonObject o = s.toObject();
	if (o.contains(QStringLiteral("error")))
	{
		m_services->setText(o.value(QStringLiteral("error")).toString().toHtmlEscaped());
		return;
	}
	const QColor muted = palette().color(QPalette::Disabled, QPalette::WindowText);
	QStringList parts;
	for (const QJsonValue& v : o.value(QStringLiteral("list")).toArray())
	{
		const QJsonObject e = v.toObject();
		const bool open = e.value(QStringLiteral("open")).toBool();
		const QString label = QStringLiteral("%1 %2").arg(e.value(QStringLiteral("name")).toString().toHtmlEscaped()).arg(e.value(QStringLiteral("port")).toInt());
		parts << (open ? QStringLiteral("<span style=\"color:#2e9e44\">●</span>&nbsp;<b>%1</b>").arg(label)
		               : QStringLiteral("<span style=\"color:%1\">○&nbsp;%2</span>").arg(muted.name(), label));
	}
	m_services->setText(parts.join(QStringLiteral("&nbsp;&nbsp;&nbsp; ")) + QStringLiteral("<br><small>%1</small>").arg(m_backend->text(QStringLiteral("ov.services.at")).arg(o.value(QStringLiteral("ip")).toString().toHtmlEscaped(), when(o.value(QStringLiteral("at")).toInteger()))));
}

void OverviewTab::renderInventory(const QJsonArray& fields)
{
	while (QLayoutItem* it = m_inventory->takeAt(0))
	{
		if (QWidget* w = it->widget())
			w->deleteLater();
		delete it;
	}
	int row = 0;
	for (const QJsonValue& v : fields)
	{
		const QJsonObject f = v.toObject();
		const QString key = f.value(QStringLiteral("key")).toString();
		auto* label = new QLabel(fieldLabel(key), m_inventoryBox);
		auto* edit = new QLineEdit(f.value(QStringLiteral("value")).toString(), m_inventoryBox);
		edit->setPlaceholderText(m_backend->text(QStringLiteral("ov.field.unknown")));
		label->setBuddy(edit);
		QString src = f.value(QStringLiteral("source")).toString();
		if (const QString w = when(f.value(QStringLiteral("at")).toInteger()); !w.isEmpty() && !src.isEmpty())
			src += QStringLiteral(", ") + w;
		auto* source = new QLabel(src, m_inventoryBox);
		source->setEnabled(false); // gedämpft
		m_inventory->addWidget(label, row, 0);
		m_inventory->addWidget(edit, row, 1);
		m_inventory->addWidget(source, row, 2);
		const QString before = edit->text();
		connect(edit, &QLineEdit::editingFinished, this, [this, edit, key, before] {
			if (edit->text().trimmed() == before)
				return;
			const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("ov_field") }, { QStringLiteral("field"), key }, { QStringLiteral("value"), edit->text() } });
			if (r.contains(QStringLiteral("error")))
				m_queryState->setText(r.value(QStringLiteral("error")).toString().toHtmlEscaped());
			else
				QTimer::singleShot(0, this, [this, inv = r.value(QStringLiteral("inventory")).toArray()] { renderInventory(inv); });
		});
		++row;
		const QJsonObject p = f.value(QStringLiteral("pending")).toObject();
		if (!p.isEmpty())
		{
			auto* box = new QWidget(m_inventoryBox);
			auto* h = new QHBoxLayout(box);
			h->setContentsMargins(0, 0, 0, 0);
			auto* note = new QLabel(m_backend->text(QStringLiteral("ov.pending")).arg(p.value(QStringLiteral("value")).toString(), p.value(QStringLiteral("source")).toString(), when(p.value(QStringLiteral("at")).toInteger())), box);
			note->setObjectName(QStringLiteral("fieldError"));
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
			m_inventory->addWidget(box, row, 1, 1, 2);
			++row;
		}
	}
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
	m_queryState->setText(html);
}
