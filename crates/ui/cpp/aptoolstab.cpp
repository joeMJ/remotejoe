#include "aptoolstab.h"

#include "cards.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDateTime>
#include <QtCore/QDir>
#include <QtCore/QFileInfo>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QTimer>
#include <QtCore/QUrl>
#include <QtGui/QDesktopServices>
#include <QtGui/QFontDatabase>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QScrollBar>
#include <QtWidgets/QVBoxLayout>

ApToolsTab::ApToolsTab(RjBackend* backend, const QString& id, const QString& name, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
  , m_id(id)
  , m_name(name)
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
	auto* caption = new QLabel(T("aptools.head").arg(name), head);
	QFont tf = caption->font();
	tf.setPointSizeF(tf.pointSizeF() * 1.3);
	tf.setBold(true);
	caption->setFont(tf);
	hv->addWidget(caption);
	m_state = new QLabel(T("aptools.intro"), head);
	m_state->setObjectName(QStringLiteral("rjMuted"));
	m_state->setWordWrap(true);
	hv->addWidget(m_state);
	hh->addLayout(hv, 1);
	m_start = new QPushButton(T("aptools.start"), head);
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
	const QJsonObject r = call({ { QStringLiteral("op"), QStringLiteral("aptools_run") }, { QStringLiteral("id"), m_id } });
	if (r.contains(QStringLiteral("error")))
	{
		m_state->setText(r.value(QStringLiteral("error")).toString());
		return;
	}
	m_output->clear();
	m_shownResults = -1;
	m_state->setText(m_backend->text(QStringLiteral("aptools.running")).arg(r.value(QStringLiteral("started")).toInt()));
	m_start->setEnabled(false);
	m_stop->setEnabled(true);
	m_poll->start();
}

void ApToolsTab::poll()
{
	const QJsonObject run = call({ { QStringLiteral("op"), QStringLiteral("aptools_output") } }).value(QStringLiteral("run")).toObject();
	// Nur den Lauf dieses Ziels bzw. Ordners zeigen (ein früherer Lauf für ein anderes gehört nicht hierher).
	if (run.isEmpty() || run.value(QStringLiteral("id")).toString() != m_id)
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

void ApToolsTab::showResults(const QJsonArray& results)
{
	m_shownResults = int(results.size());
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
		auto* l = new QLabel(QDir::toNativeSeparators(dir), this);
		l->setObjectName(QStringLiteral("rjMuted"));
		l->setTextInteractionFlags(Qt::TextSelectableByMouse);
		l->setWordWrap(true);
		l->setMinimumWidth(0);
		l->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
		l->setToolTip(QDir::toNativeSeparators(dir));
		row->addWidget(l, 1);
		auto* openDir = new QPushButton(m_backend->text(QStringLiteral("aptools.folder")), this);
		connect(openDir, &QPushButton::clicked, this, [dir] { QDesktopServices::openUrl(QUrl::fromLocalFile(dir)); });
		row->addWidget(openDir);
		for (const QJsonValue& f : r.value(QStringLiteral("files")).toArray())
		{
			const QString file = f.toString();
			auto* b = new QPushButton(QFileInfo(file).fileName(), this);
			b->setToolTip(m_backend->text(QStringLiteral("aptools.open.tip")));
			connect(b, &QPushButton::clicked, this, [file] { QDesktopServices::openUrl(QUrl::fromLocalFile(file)); });
			row->addWidget(b);
		}
		m_resultsBox->addLayout(row);
	}
}
