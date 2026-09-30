#ifndef OSM_CONSUMER_INTERFACE_H
#define OSM_CONSUMER_INTERFACE_H

#include <QObject>
#include <QString>

#include "interface.h"

// Marker interface for the consumer example's view plugin. All logic lives in
// OsmConsumerPlugin; this header supplies the IID used by Q_PLUGIN_METADATA
// and Q_DECLARE_INTERFACE.
class OsmConsumerInterface : public PluginInterface
{
public:
    virtual ~OsmConsumerInterface() = default;
};

#define OsmConsumerInterface_iid "org.logos.OsmConsumerInterface/1"
Q_DECLARE_INTERFACE(OsmConsumerInterface, OsmConsumerInterface_iid)

#endif // OSM_CONSUMER_INTERFACE_H
