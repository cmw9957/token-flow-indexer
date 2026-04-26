create table if not exists chains (
    chain_id integer primary key,
    name varchar(50) not null
);

create table if not exists blocks (
    chain_id integer not null references chains(chain_id),
    block_number bigint not null,
    block_hash varchar(66) not null,
    parent_hash varchar(66) not null,
    block_timestamp timestamp not null,
    tx_count integer not null default 0,
    movement_count integer not null default 0,
    indexed_at timestamp not null default now(),

    primary key (chain_id, block_number),
    unique (chain_id, block_hash)
);

create index if not exists blocks_parent_hash_idx
    on blocks (chain_id, parent_hash);

create table if not exists asset_movements (
    chain_id integer not null,
    block_number bigint not null,
    block_hash varchar(66) not null,
    block_timestamp timestamp not null,
    tx_hash varchar(66) not null,
    tx_index integer not null,
    source_type varchar(20) not null,
    asset_type varchar(20) not null,
    token_address varchar(42),
    from_address varchar(42) not null,
    to_address varchar(42),
    token_id numeric(78, 0),
    amount_raw numeric(78, 0) not null,
    log_index integer,
    log_sub_index integer not null default 0,
    created_at timestamp not null default now(),

    foreign key (chain_id, block_number)
        references blocks(chain_id, block_number)
        on delete cascade,

    check (source_type in ('TX_VALUE', 'LOG')),
    check (asset_type in ('NATIVE', 'ERC20', 'ERC721', 'ERC1155', 'UNKNOWN')),
    check (amount_raw >= 0)
);

create unique index if not exists asset_movements_tx_value_uid
    on asset_movements (chain_id, tx_hash)
    where source_type = 'TX_VALUE';

create unique index if not exists asset_movements_log_uid
    on asset_movements (chain_id, tx_hash, log_index, log_sub_index)
    where source_type = 'LOG';

create index if not exists asset_movements_block_idx
    on asset_movements (chain_id, block_number);

create index if not exists asset_movements_block_hash_idx
    on asset_movements (chain_id, block_hash);

create index if not exists asset_movements_token_block_idx
    on asset_movements (chain_id, token_address, block_number);

create index if not exists asset_movements_from_block_idx
    on asset_movements (chain_id, from_address, block_number);

create index if not exists asset_movements_to_block_idx
    on asset_movements (chain_id, to_address, block_number);

create table if not exists sync_checkpoints (
    chain_id integer not null references chains(chain_id),
    last_indexed_block bigint,
    last_indexed_hash varchar(66),
    status varchar(20) not null default 'IDLE',
    updated_at timestamp not null default now(),

    primary key (chain_id)
);
