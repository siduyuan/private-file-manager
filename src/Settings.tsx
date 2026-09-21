import { useState, useEffect } from 'react';
import {
  Modal, Tabs, Button, Input, Space, List, Tag, message, Popconfirm,
  Form, Select, Divider, Typography, Card, Descriptions, Tooltip,
} from 'antd';
import {
  DatabaseOutlined, PlusOutlined, LinkOutlined, DeleteOutlined,
  EditOutlined, LockOutlined, FolderOpenOutlined, CheckCircleOutlined,
  CloudOutlined, UsbOutlined, HomeOutlined, SettingOutlined,
  SafetyOutlined, ReloadOutlined,
} from '@ant-design/icons';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';

const { Text, Title } = Typography;
const { Password } = Input;

interface DbConnectionInfo {
  uuid: string;
  display_name: string;
  db_type: string;
  path: string;
  is_connected: boolean;
  is_default: boolean;
  has_password: boolean;
}

interface SettingsProps {
  open: boolean;
  onClose: () => void;
  onConnectionsChanged: () => void;
  onSwitchDatabase: (uuid: string) => void;
}

const DB_TYPE_MAP: Record<string, { label: string; color: string; icon: React.ReactNode }> = {
  local: { label: '本地', color: 'default', icon: <HomeOutlined /> },
  cloud: { label: '云端', color: 'blue', icon: <CloudOutlined /> },
  usb: { label: 'U盘', color: 'orange', icon: <UsbOutlined /> },
};

export default function Settings({ open, onClose, onConnectionsChanged, onSwitchDatabase }: SettingsProps) {
  const [connections, setConnections] = useState<DbConnectionInfo[]>([]);
  const [activeTab, setActiveTab] = useState('databases');
  const [defaultDbPath, setDefaultDbPath] = useState('');

  // 新建数据库
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [createPath, setCreatePath] = useState('');
  const [createName, setCreateName] = useState('');
  const [createPassword, setCreatePassword] = useState('');
  const [createType, setCreateType] = useState('local');

  // 连接数据库
  const [connectModalOpen, setConnectModalOpen] = useState(false);
  const [connectPath, setConnectPath] = useState('');
  const [connectPassword, setConnectPassword] = useState('');

  // 密码管理
  const [passwordModalOpen, setPasswordModalOpen] = useState(false);
  const [passwordTarget, setPasswordTarget] = useState<DbConnectionInfo | null>(null);
  const [oldPassword, setOldPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');

  // 重命名
  const [renameModalOpen, setRenameModalOpen] = useState(false);
  const [renameTarget, setRenameTarget] = useState<DbConnectionInfo | null>(null);
  const [newName, setNewName] = useState('');

  // 修改默认数据库路径
  const [newDefaultPath, setNewDefaultPath] = useState('');

  useEffect(() => {
    if (open) {
      loadConnections();
    }
  }, [open]);

  const loadConnections = async () => {
    try {
      const conns = await invoke<DbConnectionInfo[]>('list_connections');
      setConnections(conns);
      const defaultConn = conns.find(c => c.is_default);
      if (defaultConn) {
        setDefaultDbPath(defaultConn.path);
        setNewDefaultPath(defaultConn.path);
      }
      onConnectionsChanged();
    } catch (e) {
      message.error(`加载数据库列表失败: ${e}`);
    }
  };

  // 新建数据库
  const handleCreate = async () => {
    if (!createPath) {
      message.warning('请选择数据库存储位置');
      return;
    }
    try {
      await invoke('create_database', {
        request: {
          path: createPath,
          password: createPassword || null,
          display_name: createName || null,
          db_type: createType,
        },
      });
      message.success('数据库创建成功');
      setCreateModalOpen(false);
      setCreatePath('');
      setCreateName('');
      setCreatePassword('');
      setCreateType('local');
      loadConnections();
    } catch (e) {
      message.error(`创建失败: ${e}`);
    }
  };

  // 连接已有数据库
  const handleConnect = async () => {
    if (!connectPath) {
      message.warning('请选择数据库目录');
      return;
    }
    try {
      await invoke('connect_database', {
        path: connectPath,
        password: connectPassword || null,
      });
      message.success('数据库连接成功');
      setConnectModalOpen(false);
      setConnectPath('');
      setConnectPassword('');
      loadConnections();
    } catch (e) {
      message.error(`连接失败: ${e}`);
    }
  };

  // 移除数据库
  const handleRemove = async (uuid: string) => {
    try {
      await invoke('remove_database', { uuid });
      message.success('已移除');
      loadConnections();
    } catch (e) {
      message.error(`移除失败: ${e}`);
    }
  };

  // 切换数据库
  const handleSwitch = async (uuid: string) => {
    try {
      await invoke('switch_database', { uuid });
      message.success('已切换数据库');
      loadConnections();
      onSwitchDatabase(uuid);
    } catch (e) {
      message.error(`切换失败: ${e}`);
    }
  };

  // 设置/修改密码
  const handleSetPassword = async () => {
    if (!passwordTarget) return;
    try {
      if (passwordTarget.has_password) {
        await invoke('reset_database_password', {
          uuid: passwordTarget.uuid,
          old_password: oldPassword,
          new_password: newPassword,
        });
      } else {
        await invoke('set_database_password', {
          uuid: passwordTarget.uuid,
          password: newPassword || null,
        });
      }
      message.success('密码已更新');
      setPasswordModalOpen(false);
      setOldPassword('');
      setNewPassword('');
      loadConnections();
    } catch (e) {
      message.error(`密码操作失败: ${e}`);
    }
  };

  // 重命名数据库
  const handleRename = async () => {
    if (!renameTarget || !newName) return;
    try {
      await invoke('rename_database', {
        uuid: renameTarget.uuid,
        new_name: newName,
      });
      message.success('重命名成功');
      setRenameModalOpen(false);
      loadConnections();
    } catch (e) {
      message.error(`重命名失败: ${e}`);
    }
  };

  // 修改默认数据库路径
  const handleChangeDefaultPath = async () => {
    if (!newDefaultPath || newDefaultPath === defaultDbPath) return;
    try {
      await invoke('change_default_db_path', { new_path: newDefaultPath });
      message.success('默认数据库路径已更新');
      setDefaultDbPath(newDefaultPath);
      loadConnections();
    } catch (e) {
      message.error(`路径修改失败: ${e}`);
    }
  };

  // 完整性检查
  const handleIntegrityCheck = async (uuid: string) => {
    try {
      const result = await invoke<{ valid: boolean; details: string[] }>('check_integrity', { uuid });
      if (result.valid) {
        message.success('数据库完整性检查通过');
      } else {
        message.warning(`发现问题: ${result.details.join(', ')}`);
      }
    } catch (e) {
      message.error(`检查失败: ${e}`);
    }
  };

  const selectFolder = async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    return selected as string | null;
  };

  const openPasswordModal = (conn: DbConnectionInfo) => {
    setPasswordTarget(conn);
    setOldPassword('');
    setNewPassword('');
    setPasswordModalOpen(true);
  };

  const openRenameModal = (conn: DbConnectionInfo) => {
    setRenameTarget(conn);
    setNewName(conn.display_name);
    setRenameModalOpen(true);
  };

  return (
    <Modal
      title={
        <Space>
          <SettingOutlined />
          设置
        </Space>
      }
      open={open}
      onCancel={onClose}
      footer={null}
      width={720}
      styles={{ body: { minHeight: 400 } }}
    >
      <Tabs activeKey={activeTab} onChange={setActiveTab} items={[
        {
          key: 'databases',
          label: <span><DatabaseOutlined /> 数据库管理</span>,
          children: (
            <div>
              <Space style={{ marginBottom: 16 }}>
                <Button type="primary" icon={<PlusOutlined />} onClick={() => setCreateModalOpen(true)}>
                  新建数据库
                </Button>
                <Button icon={<LinkOutlined />} onClick={() => setConnectModalOpen(true)}>
                  连接已有数据库
                </Button>
                <Button icon={<ReloadOutlined />} onClick={loadConnections}>
                  刷新
                </Button>
              </Space>

              <List
                dataSource={connections}
                renderItem={(conn) => {
                  const typeInfo = DB_TYPE_MAP[conn.db_type] || DB_TYPE_MAP.local;
                  return (
                    <Card
                      size="small"
                      style={{
                        marginBottom: 8,
                        borderColor: conn.is_connected ? '#52c41a' : undefined,
                      }}
                      styles={{ body: { padding: '12px 16px' } }}
                    >
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                        <div>
                          <Space>
                            <Text strong style={{ fontSize: 15 }}>{conn.display_name}</Text>
                            <Tag icon={typeInfo.icon} color={typeInfo.color}>{typeInfo.label}</Tag>
                            {conn.is_default && <Tag color="green">默认</Tag>}
                            {conn.is_connected && <Tag icon={<CheckCircleOutlined />} color="success">当前</Tag>}
                            {conn.has_password && <Tag icon={<LockOutlined />} color="warning">已加密</Tag>}
                          </Space>
                          <div>
                            <Text type="secondary" style={{ fontSize: 12 }}>{conn.path}</Text>
                          </div>
                        </div>
                        <Space>
                          {!conn.is_connected && !conn.is_default && (
                            <Tooltip title="切换到此数据库">
                              <Button size="small" type="primary" onClick={() => handleSwitch(conn.uuid)}>
                                切换
                              </Button>
                            </Tooltip>
                          )}
                          <Tooltip title="重命名">
                            <Button size="small" icon={<EditOutlined />} onClick={() => openRenameModal(conn)} />
                          </Tooltip>
                          <Tooltip title={conn.has_password ? '修改密码' : '设置密码'}>
                            <Button size="small" icon={<LockOutlined />} onClick={() => openPasswordModal(conn)} />
                          </Tooltip>
                          <Tooltip title="完整性检查">
                            <Button size="small" icon={<SafetyOutlined />} onClick={() => handleIntegrityCheck(conn.uuid)} />
                          </Tooltip>
                          {!conn.is_default && (
                            <Popconfirm title="确定移除此数据库？（不会删除文件）" onConfirm={() => handleRemove(conn.uuid)}>
                              <Tooltip title="移除">
                                <Button size="small" danger icon={<DeleteOutlined />} />
                              </Tooltip>
                            </Popconfirm>
                          )}
                        </Space>
                      </div>
                    </Card>
                  );
                }}
              />
            </div>
          ),
        },
        {
          key: 'general',
          label: <span><SettingOutlined /> 常规设置</span>,
          children: (
            <div>
              <Title level={5}>默认数据库</Title>
              <Descriptions column={1} bordered size="small">
                <Descriptions.Item label="存储路径">
                  <Space>
                    <Text code style={{ fontSize: 12 }}>{defaultDbPath}</Text>
                  </Space>
                </Descriptions.Item>
                <Descriptions.Item label="更改路径">
                  <Space>
                    <Input
                      value={newDefaultPath}
                      onChange={e => setNewDefaultPath(e.target.value)}
                      style={{ width: 360 }}
                      placeholder="选择新的存储路径"
                    />
                    <Button icon={<FolderOpenOutlined />} onClick={async () => {
                      const dir = await selectFolder();
                      if (dir) setNewDefaultPath(dir);
                    }} />
                    <Button type="primary" onClick={handleChangeDefaultPath}
                      disabled={!newDefaultPath || newDefaultPath === defaultDbPath}>
                      迁移
                    </Button>
                  </Space>
                </Descriptions.Item>
              </Descriptions>

              <Divider />

              <Title level={5}>关于</Title>
              <Descriptions column={1} bordered size="small">
                <Descriptions.Item label="应用名称">私有文件管理器</Descriptions.Item>
                <Descriptions.Item label="版本">0.1.0</Descriptions.Item>
                <Descriptions.Item label="存储方式">二进制分片存储，文件不以原始形态暴露</Descriptions.Item>
              </Descriptions>
            </div>
          ),
        },
      ]} />

      {/* 新建数据库弹窗 */}
      <Modal
        title="新建数据库"
        open={createModalOpen}
        onOk={handleCreate}
        onCancel={() => setCreateModalOpen(false)}
        okText="创建"
      >
        <Form layout="vertical">
          <Form.Item label="存储位置" required>
            <Space>
              <Input value={createPath} onChange={e => setCreatePath(e.target.value)} style={{ width: 400 }} placeholder="选择数据库存储目录" />
              <Button icon={<FolderOpenOutlined />} onClick={async () => {
                const dir = await selectFolder();
                if (dir) setCreatePath(dir);
              }} />
            </Space>
          </Form.Item>
          <Form.Item label="显示名称">
            <Input value={createName} onChange={e => setCreateName(e.target.value)} placeholder="留空则自动生成" />
          </Form.Item>
          <Form.Item label="密码">
            <Password value={createPassword} onChange={e => setCreatePassword(e.target.value)} placeholder="留空则不设密码" />
          </Form.Item>
          <Form.Item label="类型">
            <Select value={createType} onChange={setCreateType} style={{ width: 120 }}>
              <Select.Option value="local">本地</Select.Option>
              <Select.Option value="usb">U盘</Select.Option>
              <Select.Option value="cloud">云端</Select.Option>
            </Select>
          </Form.Item>
        </Form>
      </Modal>

      {/* 连接已有数据库弹窗 */}
      <Modal
        title="连接已有数据库"
        open={connectModalOpen}
        onOk={handleConnect}
        onCancel={() => setConnectModalOpen(false)}
        okText="连接"
      >
        <Form layout="vertical">
          <Form.Item label="数据库目录" required>
            <Space>
              <Input value={connectPath} onChange={e => setConnectPath(e.target.value)} style={{ width: 400 }} placeholder="选择包含 metadata.db 的目录" />
              <Button icon={<FolderOpenOutlined />} onClick={async () => {
                const dir = await selectFolder();
                if (dir) setConnectPath(dir);
              }} />
            </Space>
          </Form.Item>
          <Form.Item label="密码（如有）">
            <Password value={connectPassword} onChange={e => setConnectPassword(e.target.value)} placeholder="如果数据库有密码请输入" />
          </Form.Item>
        </Form>
      </Modal>

      {/* 密码管理弹窗 */}
      <Modal
        title={passwordTarget?.has_password ? '修改密码' : '设置密码'}
        open={passwordModalOpen}
        onOk={handleSetPassword}
        onCancel={() => setPasswordModalOpen(false)}
        okText="确定"
      >
        <Form layout="vertical">
          {passwordTarget?.has_password && (
            <Form.Item label="原密码" required>
              <Password value={oldPassword} onChange={e => setOldPassword(e.target.value)} />
            </Form.Item>
          )}
          <Form.Item label="新密码" required>
            <Password value={newPassword} onChange={e => setNewPassword(e.target.value)} placeholder="留空则移除密码" />
          </Form.Item>
        </Form>
      </Modal>

      {/* 重命名弹窗 */}
      <Modal
        title="重命名数据库"
        open={renameModalOpen}
        onOk={handleRename}
        onCancel={() => setRenameModalOpen(false)}
        okText="确定"
      >
        <Form layout="vertical">
          <Form.Item label="显示名称" required>
            <Input value={newName} onChange={e => setNewName(e.target.value)} />
          </Form.Item>
        </Form>
      </Modal>
    </Modal>
  );
}
