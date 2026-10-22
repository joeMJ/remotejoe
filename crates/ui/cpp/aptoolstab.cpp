#include "aptoolstab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QDir>
#include <QtCore/QFileInfo>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QRegularExpression>
#include <QtCore/QTimer>
#include <QtCore/QUrl>
#include <QtGui/QDesktopServices>
#include <QtGui/QFontDatabase>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QDialog>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QScrollBar>
#include <QtWidgets/QVBoxLayout>

ApToolsTab::ApToolsTab(RjBackend* backend, const QString& id, const QString& name, const QString& tool, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_id(id)
  , m_name(name)
  , m_tool(tool)
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
	m_state = new QLabel(T(tool == QLatin1String("inventar") ? "aptools.intro.inventar" : "aptools.intro"), head);
	m_state->setObjectName(QStringLiteral("rjMuted"));
	m_state->setWordWrap(true);
	hv->addWidget(m_state);
	hh->addLayout(hv, 1);
	m_start = new QPushButton(T(tool == QLatin1String("inventar") ? "aptools.start.inventar" : "aptools.start"), head);
	m_start->setDefault(true);
	m_stop = new QPushButton(T("aptools.stop"), head);
	auto* folder = new QPushButton(T("aptools.folder"), head);
	auto* close = new QPushButton(T("mass.close"), head);
	for (QPushButton* b : { m_start, m_stop, folder, close })
		hh->addWidget(b, 0, Qt::AlignTop);
	hin->addLayout(hh);
	lay->addWidget(head);

	QVBoxLayout* sin = nullptr;
	auto* st = rjui::card(this, &sin);
	sin->addWidget(rjui::section(T("aptools.setup.title"), st));
	auto* sh = new QHBoxLayout;
	auto* sv = new QVBoxLayout;
	m_collection = new QLabel(st);
	m_collection->setWordWrap(true);
	m_python = new QLabel(st);
	m_python->setWordWrap(true);
	sv->addWidget(m_collection);
	sv->addWidget(m_python);
	sh->addLayout(sv, 1);
	m_setup = new QPushButton(T("aptools.setup"), st);
	sh->addWidget(m_setup, 0, Qt::AlignTop);
	sin->addLayout(sh);
	lay->addWidget(st);

	QVBoxLayout* oin = nullptr;
	auto* oc = rjui::card(this, &oin);
	oin->addWidget(rjui::section(T("aptools.output"), oc));
	m_output = new QPlainTextEdit(oc);
	m_output->setReadOnly(true);
	m_output->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
	m_output->setLineWrapMode(QPlainTextEdit::NoWrap);
	oin->addWidget(m_output, 1);
	oin->addWidget(rjui::section(T("aptools.results"), oc));
	m_resultsBox = new QVBoxLayout;
	oin->addLayout(m_resultsBox);
	lay->addWidget(oc, 1);

	m_poll = new QTimer(this);
	m_poll->setInterval(400);
	connect(m_poll, &QTimer::timeout, this, &ApToolsTab::poll);
	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(m_setup, &QPushButton::clicked, this, [this] { setup(false); });
	connect(m_start, &QPushButton::clicked, this, &ApToolsTab::start);
	connect(m_stop, &QPushButton::clicked, this, [this] { call({ { QStringLiteral("op"), QStringLiteral("aptools_stop") } }); });
	connect(folder, &QPushButton::clicked, this, [this] {
		const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("aptools_folder") }, { QStringLiteral("id"), m_id } });
		const QString p = r.value(QStringLiteral("path")).toString();
		if (p.isEmpty())
			return;
		QDir().mkpath(p);
		QDesktopServices::openUrl(QUrl::fromLocalFile(p));
	});
	refreshStatus();
	poll();
}

QString ApToolsTab::title() const
{
	return m_backend->text(QString::fromLatin1(m_tool == QLatin1String("inventar") ? "aptools.tab.inventar" : "aptools.tab")).arg(m_name);
}

QJsonObject ApToolsTab::call(const QJsonObject& cmd) const
{
	return QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
}

void ApToolsTab::refreshStatus()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	const QJsonObject s = call({ { QStringLiteral("op"), QStringLiteral("aptools_status") } });
	if (s.contains(QStringLiteral("error")))
	{
		m_collection->setText(s.value(QStringLiteral("error")).toString());
		m_python->clear();
		m_setup->setEnabled(false);
		m_start->setEnabled(false);
		return;
	}
	const bool installed = s.value(QStringLiteral("installed")).toBool();
	if (installed)
	{
		const QString commit = s.value(QStringLiteral("commit")).toString().left(7);
		const QString at = QDateTime::fromSecsSinceEpoch(s.value(QStringLiteral("at")).toInteger()).toString(QStringLiteral("dd.MM.yyyy"));
		m_collection->setText(T("aptools.collection").arg(s.value(QStringLiteral("version")).toString(QStringLiteral("?")), commit.isEmpty() ? QStringLiteral("?") : commit, at));
	}
	else
		m_collection->setText(T("aptools.collection.none"));
	const bool venv = s.value(QStringLiteral("venv")).toBool();
	const QString py = s.value(QStringLiteral("python")).toString();
	m_python->setText(venv ? T("aptools.python.venv").arg(py.isEmpty() ? QStringLiteral("?") : py)
	                  : py.isEmpty() ? T("aptools.python.none")
	                                 : T("aptools.python.found").arg(py));
	m_setup->setText(T(installed && venv ? "aptools.update" : "aptools.setup"));
	m_setup->setEnabled(true);
	const bool running = s.value(QStringLiteral("running")).toBool();
	m_start->setEnabled(installed && venv && !running);
	m_stop->setEnabled(running);
}

void ApToolsTab::setup(bool installPython)
{
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("aptools_setup") }, { QStringLiteral("python"), installPython } });
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(r.value(QStringLiteral("error")).toString());
		return;
	}
	m_setup->setEnabled(false);
	m_start->setEnabled(false);
	m_state->setText(m_backend->text(QString::fromLatin1(installPython ? "aptools.setup.python.busy" : "aptools.setup.busy")));
}

void ApToolsTab::handleSetup(const QJsonObject& r)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	if (r.value(QStringLiteral("error")).isString())
		m_state->setText(T("aptools.setup.failed").arg(r.value(QStringLiteral("error")).toString()));
	else if (r.value(QStringLiteral("need_python")).toBool())
	{
		const bool canInstall = call({ { QStringLiteral("op"), QStringLiteral("aptools_status") } }).value(QStringLiteral("can_install_python")).toBool();
		if (canInstall)
		{
			// Eigene Knopftexte: ohne Qt-Übersetzungen stünde dort „Yes“/„No“.
			QMessageBox box(QMessageBox::Question, T("aptools.python.ask.title"), T("aptools.python.ask"), QMessageBox::NoButton, this);
			auto* yes = box.addButton(T("aptools.python.ask.yes"), QMessageBox::AcceptRole);
			box.setDefaultButton(box.addButton(T("aptools.python.ask.no"), QMessageBox::RejectRole));
			box.exec();
			if (box.clickedButton() == yes)
			{
				setup(true);
				return;
			}
		}
		m_state->setText(T(canInstall ? "aptools.python.missing.win" : "aptools.python.missing"));
	}
	else
		m_state->setText(T("aptools.setup.done").arg(r.value(QStringLiteral("paramiko")).toString()));
	refreshStatus();
}

void ApToolsTab::start()
{
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("aptools_run") }, { QStringLiteral("id"), m_id }, { QStringLiteral("tool"), m_tool } });
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(r.value(QStringLiteral("error")).toString());
		return;
	}
	m_output->clear();
	m_shownResults = -1;
	m_state->setText(m_backend->text(QStringLiteral("aptools.running")).arg(m_tool == QLatin1String("inventar") ? QStringLiteral("ap_list") : QStringLiteral("ap_check")).arg(r.value(QStringLiteral("started")).toInt()));
	m_start->setEnabled(false);
	m_stop->setEnabled(true);
	m_poll->start();
}

void ApToolsTab::poll()
{
	const QJsonObject run = call({ { QStringLiteral("op"), QStringLiteral("aptools_output") } }).value(QStringLiteral("run")).toObject();
	// Nur den Lauf dieses Ziels bzw. Ordners und Werkzeugs zeigen (ein früherer Lauf gehört nicht hierher).
	if (run.isEmpty() || run.value(QStringLiteral("id")).toString() != m_id || run.value(QStringLiteral("tool")).toString() != m_tool)
	{
		m_poll->stop();
		return;
	}
	const QString text = run.value(QStringLiteral("text")).toString();
	if (text != m_output->toPlainText())
	{
		auto* sb = m_output->verticalScrollBar();
		const bool atEnd = sb->value() >= sb->maximum() - 2;
		m_output->setPlainText(text);
		if (atEnd)
			sb->setValue(sb->maximum());
	}
	const QJsonArray results = run.value(QStringLiteral("results")).toArray();
	if (results.size() != m_shownResults)
		showResults(results);
	if (run.value(QStringLiteral("done")).toBool())
	{
		if (m_poll->isActive())
			m_state->setText(run.value(QStringLiteral("stopped")).toBool() ? m_backend->text(QStringLiteral("aptools.stopped"))
			                                                               : m_backend->text(QStringLiteral("aptools.done")).arg(results.size()));
		m_poll->stop();
		refreshStatus();
	}
	else if (!m_poll->isActive())
		m_poll->start();
}

namespace
{
/// Ergebniszeilen: Ordner (umbrechend, voller Pfad im Tooltip), „Ordner öffnen“ und je Datei ein Knopf.
void fillResults(RjBackend* backend, QWidget* owner, QVBoxLayout* box, const QJsonArray& results);
} // namespace

void ApToolsTab::showResults(const QJsonArray& results)
{
	m_shownResults = int(results.size());
	fillResults(m_backend, this, m_resultsBox, results);
}

namespace
{
void fillResults(RjBackend* backend, QWidget* owner, QVBoxLayout* m_resultsBox, const QJsonArray& results)
{
	QWidget* self = owner;
	RjBackend* m_backend = backend;
	while (QLayoutItem* it = m_resultsBox->takeAt(0))
	{
		if (QWidget* w = it->widget())
			w->deleteLater();
		else if (QLayout* l = it->layout())
		{
			while (QLayoutItem* c = l->takeAt(0))
			{
				if (c->widget())
					c->widget()->deleteLater();
				delete c;
			}
		}
		delete it;
	}
	for (const QJsonValue& v : results)
	{
		const QJsonObject r = v.toObject();
		auto* row = new QHBoxLayout;
		const QString dir = r.value(QStringLiteral("path")).toString();
		// Langer Pfad: umbrechen statt das Fenster zu verbreitern; vollständig im Tooltip.
		auto* l = new QLabel(QDir::toNativeSeparators(dir), self);
		l->setObjectName(QStringLiteral("rjMuted"));
		l->setTextInteractionFlags(Qt::TextSelectableByMouse);
		l->setWordWrap(true);
		l->setMinimumWidth(0);
		l->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
		l->setToolTip(QDir::toNativeSeparators(dir));
		row->addWidget(l, 1);
		auto* openDir = new QPushButton(m_backend->text(QStringLiteral("aptools.folder")), self);
		QObject::connect(openDir, &QPushButton::clicked, self, [dir] { QDesktopServices::openUrl(QUrl::fromLocalFile(dir)); });
		row->addWidget(openDir);
		for (const QJsonValue& f : r.value(QStringLiteral("files")).toArray())
		{
			const QString file = f.toString();
			auto* b = new QPushButton(QFileInfo(file).fileName(), self);
			b->setToolTip(m_backend->text(QStringLiteral("aptools.open.tip")));
			QObject::connect(b, &QPushButton::clicked, self, [file] { QDesktopServices::openUrl(QUrl::fromLocalFile(file)); });
			row->addWidget(b);
		}
		m_resultsBox->addLayout(row);
	}
}
} // namespace

// ---------- ArubaWriteTab ----------

bool ArubaWriteTab::confirm(RjBackend* backend, QWidget* parent, const QString& id, const QString& name, const QString& tool, QString* url, bool* reboot)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	const bool update = tool == QLatin1String("update");
	const QJsonObject r = QJsonDocument::fromJson(backend->edit(QString::fromUtf8(QJsonDocument(QJsonObject{ { QStringLiteral("op"), QStringLiteral("aptools_targets") }, { QStringLiteral("id"), id } }).toJson(QJsonDocument::Compact))).toUtf8()).object();
	if (r.contains(QStringLiteral("error")))
	{
		QMessageBox::warning(parent, T(update ? "aruba.update" : "aruba.reload"), r.value(QStringLiteral("error")).toString());
		return false;
	}
	if (r.value(QStringLiteral("write_running")).toBool())
	{
		QMessageBox::information(parent, T(update ? "aruba.update" : "aruba.reload"), T("aruba.write.running"));
		return false;
	}
	const QJsonArray targets = r.value(QStringLiteral("targets")).toArray();
	if (targets.isEmpty())
	{
		QMessageBox::information(parent, T(update ? "aruba.update" : "aruba.reload"), T("aruba.write.none"));
		return false;
	}
	QDialog dlg(parent);
	dlg.setMinimumWidth(560);
	dlg.setWindowTitle(T(update ? "aruba.update" : "aruba.reload") + QStringLiteral(" · ") + name);
	auto* v = new QVBoxLayout(&dlg);
	auto* intro = new QLabel(T(update ? "aruba.update.intro" : "aruba.reload.intro"), &dlg);
	intro->setWordWrap(true);
	v->addWidget(intro);
	auto* list = new QPlainTextEdit(&dlg);
	list->setReadOnly(true);
	QStringList lines;
	for (const QJsonValue& tv : targets)
	{
		const QJsonObject t = tv.toObject();
		const QJsonObject w = t.value(QStringLiteral("write")).toObject();
		const QString state = w.isEmpty() ? T("aruba.write.unknown") : w.value(QStringLiteral("ok")).toBool() ? T("aruba.write.ok") : T("aruba.write.blocked").arg(w.value(QStringLiteral("reason")).toString());
		lines << QStringLiteral("%1 (%2): %3").arg(t.value(QStringLiteral("name")).toString(), t.value(QStringLiteral("host")).toString(), state);
	}
	list->setPlainText(lines.join(QLatin1Char('\n')));
	list->setMinimumHeight(qMin(240, 40 + 20 * int(targets.size())));
	v->addWidget(list);
	QLineEdit* urlEdit = nullptr;
	QCheckBox* rebootBox = nullptr;
	QLabel* err = nullptr;
	if (update)
	{
		auto* f = new QFormLayout;
		urlEdit = new QLineEdit(&dlg);
		urlEdit->setPlaceholderText(T("aruba.update.url.ph"));
		urlEdit->setMinimumWidth(420);
		f->addRow(T("aruba.update.url"), urlEdit);
		rebootBox = new QCheckBox(T("aruba.update.reboot"), &dlg);
		f->addRow(QString(), rebootBox);
		v->addLayout(f);
		err = new QLabel(&dlg);
		err->setProperty("rjWarnText", true);
		err->setWordWrap(true);
		v->addWidget(err);
	}
	auto* note = new QLabel(T("aruba.write.note"), &dlg);
	note->setObjectName(QStringLiteral("rjMuted"));
	note->setWordWrap(true);
	v->addWidget(note);
	auto* bh = new QHBoxLayout;
	bh->addStretch(1);
	auto* go = new QPushButton(T(update ? "aruba.update" : "aruba.reload"), &dlg);
	auto* cancel = new QPushButton(T("aruba.write.cancel"), &dlg);
	cancel->setDefault(true);
	bh->addWidget(go);
	bh->addWidget(cancel);
	v->addLayout(bh);
	QObject::connect(cancel, &QPushButton::clicked, &dlg, &QDialog::reject);
	QObject::connect(go, &QPushButton::clicked, &dlg, [&] {
		if (urlEdit)
		{
			const QString u = urlEdit->text().trimmed();
			if (!(u.startsWith(QLatin1String("http://")) || u.startsWith(QLatin1String("https://"))) || u.contains(QLatin1Char(' ')))
			{
				err->setText(T("aruba.update.url.bad"));
				return;
			}
			// Ohne Versionsnummer in der URL entfällt die Prüfung „schon installiert“; einmal darauf hinweisen.
			// Nur im Pfad suchen: eine IPv4-Adresse als Rechner sähe sonst wie eine Version aus.
			if (!QRegularExpression(QStringLiteral("\\d+\\.\\d+\\.\\d+\\.\\d+")).match(QUrl(u).path()).hasMatch() && err->text() != T("aruba.update.url.noversion"))
			{
				err->setText(T("aruba.update.url.noversion"));
				return;
			}
		}
		dlg.accept();
	});
	if (dlg.exec() != QDialog::Accepted)
		return false;
	if (urlEdit)
		*url = urlEdit->text().trimmed();
	if (rebootBox)
		*reboot = rebootBox->isChecked();
	return true;
}

ArubaWriteTab::ArubaWriteTab(RjBackend* backend, const QString& id, const QString& name, const QString& tool, const QString& url, bool reboot, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_id(id)
  , m_name(name)
  , m_tool(tool)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	const bool update = tool == QLatin1String("update");
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
	auto* sub = new QLabel(update ? T("aruba.update.sub").arg(url, T(reboot ? "aruba.update.sub.reboot" : "aruba.update.sub.noreboot")) : T("aruba.reload.sub"), head);
	sub->setObjectName(QStringLiteral("rjMuted"));
	sub->setWordWrap(true);
	sub->setTextInteractionFlags(Qt::TextSelectableByMouse);
	hv->addWidget(sub);
	m_state = new QLabel(head);
	m_state->setWordWrap(true);
	hv->addWidget(m_state);
	hh->addLayout(hv, 1);
	m_stop = new QPushButton(T("aptools.stop"), head);
	auto* folder = new QPushButton(T("aptools.folder"), head);
	auto* close = new QPushButton(T("mass.close"), head);
	for (QPushButton* b : { m_stop, folder, close })
		hh->addWidget(b, 0, Qt::AlignTop);
	hin->addLayout(hh);
	lay->addWidget(head);

	QVBoxLayout* oin = nullptr;
	auto* oc = rjui::card(this, &oin);
	const QFont mono = QFontDatabase::systemFont(QFontDatabase::FixedFont);
	// Statustafel nur beim Update (ap_swarm_reload zeichnet keine).
	auto* boardTitle = rjui::section(T("aruba.write.board"), oc);
	oin->addWidget(boardTitle);
	m_board = new QPlainTextEdit(oc);
	m_board->setReadOnly(true);
	m_board->setFont(mono);
	m_board->setLineWrapMode(QPlainTextEdit::NoWrap);
	m_board->setMaximumHeight(220);
	m_board->setPlaceholderText(T("aruba.write.board.none"));
	oin->addWidget(m_board);
	boardTitle->setVisible(update);
	m_board->setVisible(update);
	oin->addWidget(rjui::section(T("aptools.output"), oc));
	m_output = new QPlainTextEdit(oc);
	m_output->setReadOnly(true);
	m_output->setFont(mono);
	m_output->setLineWrapMode(QPlainTextEdit::NoWrap);
	oin->addWidget(m_output, 1);
	oin->addWidget(rjui::section(T("aptools.results"), oc));
	m_resultsBox = new QVBoxLayout;
	oin->addLayout(m_resultsBox);
	lay->addWidget(oc, 1);

	m_poll = new QTimer(this);
	m_poll->setInterval(500);
	connect(m_poll, &QTimer::timeout, this, &ArubaWriteTab::poll);
	connect(close, &QPushButton::clicked, this, &ToolTab::closeRequested);
	connect(folder, &QPushButton::clicked, this, [this] {
		const QString p = call({ { QStringLiteral("op"), QStringLiteral("aptools_folder") }, { QStringLiteral("id"), m_id } }).value(QStringLiteral("path")).toString();
		if (p.isEmpty())
			return;
		QDir().mkpath(p);
		QDesktopServices::openUrl(QUrl::fromLocalFile(p));
	});
	connect(m_stop, &QPushButton::clicked, this, [this] {
		auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
		QMessageBox box(QMessageBox::Warning, title(), T("aruba.write.stop.ask"), QMessageBox::NoButton, this);
		auto* yes = box.addButton(T("aruba.write.stop.yes"), QMessageBox::AcceptRole);
		box.setDefaultButton(box.addButton(T("aruba.write.stop.no"), QMessageBox::RejectRole));
		box.exec();
		if (box.clickedButton() == yes)
			call({ { QStringLiteral("op"), QStringLiteral("aptools_stop") }, { QStringLiteral("write"), true } });
	});

	QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("aptools_run") }, { QStringLiteral("id"), id }, { QStringLiteral("tool"), tool } };
	if (update)
	{
		cmd.insert(QStringLiteral("url"), url);
		cmd.insert(QStringLiteral("reboot"), reboot);
	}
	const QJsonObject r = call(cmd);
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(T("aruba.write.failed").arg(r.value(QStringLiteral("error")).toString()));
		m_stop->setEnabled(false);
		return;
	}
	m_started = true;
	m_state->setText(T("aruba.write.running.n").arg(r.value(QStringLiteral("started")).toInt()));
	m_poll->start();
	poll();
}

QString ArubaWriteTab::title() const
{
	return m_backend->text(QString::fromLatin1(m_tool == QLatin1String("update") ? "aruba.update" : "aruba.reload")) + QStringLiteral(" · ") + m_name;
}

QJsonObject ArubaWriteTab::call(const QJsonObject& cmd) const
{
	return QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
}

void ArubaWriteTab::poll()
{
	const QJsonObject run = call({ { QStringLiteral("op"), QStringLiteral("aptools_output") }, { QStringLiteral("write"), true } }).value(QStringLiteral("run")).toObject();
	if (run.isEmpty() || run.value(QStringLiteral("id")).toString() != m_id || run.value(QStringLiteral("tool")).toString() != m_tool)
		return;
	const QString text = run.value(QStringLiteral("text")).toString();
	if (text != m_output->toPlainText())
	{
		auto* sb = m_output->verticalScrollBar();
		const bool atEnd = sb->value() >= sb->maximum() - 2;
		m_output->setPlainText(text);
		if (atEnd)
			sb->setValue(sb->maximum());
	}
	const QString board = run.value(QStringLiteral("board")).toString();
	if (board != m_board->toPlainText())
		m_board->setPlainText(board);
	const QJsonArray results = run.value(QStringLiteral("results")).toArray();
	if (results.size() != m_shownResults)
	{
		m_shownResults = int(results.size());
		fillResults(m_backend, this, m_resultsBox, results);
	}
	if (run.value(QStringLiteral("done")).toBool())
	{
		m_poll->stop();
		m_stop->setEnabled(false);
		m_state->setText(run.value(QStringLiteral("stopped")).toBool() ? m_backend->text(QStringLiteral("aptools.stopped"))
		                                                               : m_backend->text(QStringLiteral("aruba.write.done")).arg(results.size()));
	}
}
