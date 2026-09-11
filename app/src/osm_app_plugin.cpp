#include "osm_app_plugin.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QTimer>

#include "logos_api.h"
#include "logos_api_client.h"

OsmAppPlugin::OsmAppPlugin(QObject* parent)
    : OsmAppSimpleSource(parent)
{
}

OsmAppPlugin::~OsmAppPlugin()
{
    delete m_client;
}

void OsmAppPlugin::initLogos(LogosAPI* api)
{
    m_api = api;
    setBackend(this);
    ensureClient();
    QTimer::singleShot(0, this, [this]() { refresh(); });
}

void OsmAppPlugin::ensureClient()
{
    if (m_client || !m_api) {
        return;
    }
    // The core module is named "osm" (see module/metadata.json); this replica
    // is "osm_app". RemoteObjects routes invokeOpJson calls to the core.
    m_client = new LogosAPIClient(
        QStringLiteral("osm"),
        QStringLiteral("osm_app"),
        m_api->getTokenManager(),
        this);
}

QString OsmAppPlugin::invokeCore(const QString& method, const QVariantList& args)
{
    ensureClient();
    if (!m_client) {
        return QStringLiteral("{\"ok\":false,\"error\":\"no osm client\"}");
    }
    switch (args.size()) {
        case 0:
            return m_client->invokeRemoteMethod(QStringLiteral("osm"), method).toString();
        case 1:
            return m_client->invokeRemoteMethod(QStringLiteral("osm"), method, args[0]).toString();
        case 2:
            return m_client->invokeRemoteMethod(QStringLiteral("osm"), method, args[0], args[1]).toString();
        default:
            return QStringLiteral("{\"ok\":false,\"error\":\"unsupported argument count\"}");
    }
}

void OsmAppPlugin::refresh()
{
    const QString v = invokeCore(QStringLiteral("osmVersionJson"));
    setOsmVersion(v);
    // Surface a parse error only if the core reports not-ok.
    const QJsonDocument doc = QJsonDocument::fromJson(v.toUtf8());
    if (doc.isObject() && !doc.object().value(QLatin1String("ok")).toBool()) {
        setLastErr(doc.object().value(QLatin1String("error")).toString());
    } else {
        setLastErr(QStringLiteral(""));
    }
}

QString OsmAppPlugin::invokeOp(QString name, QString argsJson)
{
    const QString result = invokeCore(
        QStringLiteral("invokeOpJson"),
        QVariantList{ name, argsJson });
    setLastResult(result);
    // Track ok/error for the UI's lastErr binding.
    const QJsonDocument doc = QJsonDocument::fromJson(result.toUtf8());
    if (doc.isObject() && !doc.object().value(QLatin1String("ok")).toBool()) {
        setLastErr(doc.object().value(QLatin1String("error")).toString());
    } else {
        setLastErr(QStringLiteral(""));
    }
    return result;
}
