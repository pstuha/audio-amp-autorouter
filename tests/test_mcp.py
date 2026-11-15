import pytest

pytest.importorskip("mcp")


def test_mcp_server_builds():
    from amproute_studio.adapters.mcp import build_server

    server = build_server(port=8999)
    assert server is not None
