#include "osm_consumer_plugin.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTimer>

#include "logos_api.h"
#include "logos_api_client.h"

OsmConsumerPlugin::OsmConsumerPlugin(QObject* parent)
    : OsmConsumerSimpleSource(parent)
{
}

OsmConsumerPlugin::~OsmConsumerPlugin()
{
    delete m_client;
}

void OsmConsumerPlugin::initLogos(LogosAPI* api)
{
    m_api = api;
    setBackend(this);
    ensureClient();
    QTimer::singleShot(0, this, [this]() { loadRegions(); });
}

void OsmConsumerPlugin::ensureClient()
{
    if (m_client || !m_api) {
        return;
    }
    // The core module is named "osm" (see module/metadata.json); this view is
    // "osm_consumer". RemoteObjects routes the calls to the SDK module.
    m_client = new LogosAPIClient(
        QStringLiteral("osm"),
        QStringLiteral("osm_consumer"),
        m_api->getTokenManager(),
        this);
}

QString OsmConsumerPlugin::invokeSdk(const QString& op, const QString& argsJson)
{
    ensureClient();
    if (!m_client) {
        return QStringLiteral("{\"ok\":false,\"error\":\"no osm client\"}");
    }
    return m_client
        ->invokeRemoteMethod(QStringLiteral("osm"),
                             QStringLiteral("invokeOpJson"),
                             op,
                             argsJson)
        .toString();
}

QString OsmConsumerPlugin::ensureOpen()
{
    if (m_opened) {
        return QString();
    }
    // A consumer is a reader: in-memory storage keeps it offline-safe, and the
    // registry lookup reads the chain directly, so no storage node is needed.
    const QString r = invokeSdk(
        QStringLiteral("open"),
        QStringLiteral("{\"state_path\":\"osm-consumer-state.json\","
                       "\"cache_dir\":\"osm-consumer-cache\","
                       "\"memory\":true}"));
    const QJsonObject o = QJsonDocument::fromJson(r.toUtf8()).object();
    if (!o.value(QStringLiteral("ok")).toBool()) {
        return o.value(QStringLiteral("error"))
            .toString(QStringLiteral("open failed"));
    }
    m_opened = true;
    return QString();
}

QString OsmConsumerPlugin::loadRegions()
{
    const QString openErr = ensureOpen();
    if (!openErr.isEmpty()) {
        setLastErr(openErr);
        return openErr;
    }
    const QString r = invokeSdk(QStringLiteral("regions"));
    const QJsonObject o = QJsonDocument::fromJson(r.toUtf8()).object();
    if (!o.value(QStringLiteral("ok")).toBool()) {
        const QString e = o.value(QStringLiteral("error")).toString();
        setLastErr(e);
        return e;
    }
    const QJsonArray arr = o.value(QStringLiteral("result")).toObject()
                               .value(QStringLiteral("regions")).toArray();
    setRegionsJson(QString::fromUtf8(QJsonDocument(arr).toJson(QJsonDocument::Compact)));
    setReady(true);
    setLastErr(QString());
    return QString();
}

QString OsmConsumerPlugin::resolveRegion(QString regionPath)
{
    const QString openErr = ensureOpen();
    if (!openErr.isEmpty()) {
        setLastErr(openErr);
        return openErr;
    }
    const QString args = QStringLiteral("{\"region\":\"%1\"}").arg(regionPath);
    const QString r = invokeSdk(QStringLiteral("lookup"), args);
    const QJsonObject o = QJsonDocument::fromJson(r.toUtf8()).object();
    if (!o.value(QStringLiteral("ok")).toBool()) {
        // "not registered" is the honest answer for a region nobody has
        // mirrored yet, and it is not an error in this view.
        const QString e = o.value(QStringLiteral("error")).toString();
        setResolvedJson(QString());
        setLastErr(e);
        return e;
    }
    setResolvedJson(QString::fromUtf8(
        QJsonDocument(o.value(QStringLiteral("result")).toObject())
            .toJson(QJsonDocument::Compact)));
    setLastErr(QString());
    return QString();
}
