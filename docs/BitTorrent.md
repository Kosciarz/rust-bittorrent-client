# BitTorrent

BitTorrent is a peer-to-peer file sharing protocol. It allows users to directly share files
with each other without across the Internet without any central server as a middleman.

## How it works

Files are divided into small pieces. Each client in the network can either request a piece
(if it is missing it) or send a piece (if another peer requests it).

- **seeder** - a peer that has pieces ready to send out
- **leecher** - a peer requesting pieces from other peers

Initialy there is a single seeder, but once other peers obtain the file they become seeders too. The protocol tends to favour more popular content. The more peers that want a file, the more peers there will be that have the file to share. Supply scales with demand.

## Downsides

- Unpopular content can be slow and difficult to download as there are few seeders.
- Small files can take longer to download than from a regular server as there is a certain amount of overhead finding peers.
- The lack of central server can lead to a situation where all peers are almost complete but are all missing the same piece (this is rare due to algorithms used to select pieces to request).

## Components

The original specification included:

- **_.torrent_ file** - this is a small file that contains basic metadata about either a single file or a group of files included in the torrent. It specifies how the file should be broken up into pieces as well as which trackers the torrent is being tracked on.
- **tracker** - a centralized server that maintains a list of torrents with a corresponding list of peers for each one.
- **client** - a program that can create or open existing torrent files. It connects to a _tracker_ and starts _seeding_ or _leaching_ parts of a file.
