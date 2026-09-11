#ifndef OSM_APP_INTERFACE_H
#define OSM_APP_INTERFACE_H

#include <QObject>
#include <QString>

#include "interface.h"

// Marker interface for the OSM Basecamp view plugin. All logic lives in
// OsmAppPlugin; this header supplies the IID used by Q_PLUGIN_METADATA and
// Q_DECLARE_INTERFACE.
class OsmAppInterface : public PluginInterface
{
public:
    virtual ~OsmAppInterface() = default;
};

#define OsmAppInterface_iid "org.logos.OsmAppInterface/1"
Q_DECLARE_INTERFACE(OsmAppInterface, OsmAppInterface_iid)

#endif // OSM_APP_INTERFACE_H
