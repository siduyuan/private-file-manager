import { useState, useEffect, useCallback } from 'react';
import { Layout, Tree, Table, Button, Input, Space, Breadcrumb, Dropdown, message, Modal, Tooltip } from 'antd';
import {
  FolderOutlined,
  FolderOpenOutlined,
  FileOutlined,
  ImportOutlined,
  ExportOutlined,
  DeleteOutlined,
  EditOutlined,
  FolderAddOutlined,
  SearchOutlined,
  HomeOutlined,
  AppstoreOutlined,
  UnorderedListOutlined,
  ReloadOutlined,
} from '@ant-design/icons';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { DataNode } from 'antd/es/tree';
import type { MenuProps } from 'antd';
import './App.css';

interface FileInfo {
  file_id: number;
  name: string;
  ext: string | null;
  mime_type: string | null;
  size_bytes: number;
  duration_sec: number | null;
  width: number | null;
  height: number | null;
  category: string | null;
  created_at: number;
  updated_at: number;
}

interface FolderInfo {
  folder_id: number;
  name: string;
  parent_id: number | null;
  created_at: number;
}

interface ImportResult {
  success_count: number;
  fail_count: number;
  errors: string[];
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
  return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
}

function formatTime(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleString('zh-CN');
}

function getFileIcon(category: string | null) {
  switch (category) {
    case 'image': return <span style={{ color: '#52c41a' }}>🖼</span>;
    case 'video_short':
    case 'video_long': return <span style={{ color: '#1890ff' }}>🎬</span>;
    default: return <FileOutlined style={{ color: '#8c8c8c' }} />;
  }
}

function App() {
  const [folders, setFolders] = useState<FolderInfo[]>([]);
  const [files, setFiles] = useState<FileInfo[]>([]);
  const [currentFolderId, setCurrentFolderId] = useState<number>(1);
  const [selectedFileIds, setSelectedFileIds] = useState<number[]>([]);
  const [searchKeyword, setSearchKeyword] = useState('');
  const [viewMode, setViewMode] = useState<'list' | 'grid'>('list');
  const [loading, setLoading] = useState(false);
  const [breadcrumb, setBreadcrumb] = useState<{ id: number; name: string }[]>([
    { id: 1, name: '根目录' },
  ]);
  const [renameModalOpen, setRenameModalOpen] = useState(false);
  const [renameValue, setRenameValue] = useState('');
  const [renameFileId, setRenameFileId] = useState<number | null>(null);
  const [newFolderModalOpen, setNewFolderModalOpen] = useState(false);
  const [newFolderName, setNewFolderName] = useState('');

  const loadFolders = useCallback(async () => {
    try {
      const result = await invoke<FolderInfo[]>('get_folders');
      setFolders(result);
    } catch (e) {
      console.error('Failed to load folders:', e);
    }
  }, []);

  const loadFiles = useCallback(async (folderId: number) => {
    setLoading(true);
    try {
      const result = await invoke<FileInfo[]>('get_files_in_folder', { folderId });
      setFiles(result);
    } catch (e) {
      console.error('Failed to load files:', e);
      message.error('加载文件失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadFolders();
    loadFiles(currentFolderId);
  }, [currentFolderId, loadFolders, loadFiles]);

  // Build tree data from folders
  const buildTreeData = (folders: FolderInfo[]): DataNode[] => {
    const folderMap = new Map<number, FolderInfo[]>();
    folders.forEach(f => {
      const pid = f.parent_id ?? 0;
      if (!folderMap.has(pid)) folderMap.set(pid, []);
      folderMap.get(pid)!.push(f);
    });

    const buildNode = (parentId: number): DataNode[] => {
      const children = folderMap.get(parentId) || [];
      return children.map(f => ({
        key: f.folder_id,
        title: f.name,
        icon: currentFolderId === f.folder_id ? <FolderOpenOutlined /> : <FolderOutlined />,
        children: buildNode(f.folder_id),
      }));
    };

    return buildNode(0);
  };

  const handleFolderSelect = (selectedKeys: React.Key[]) => {
    if (selectedKeys.length > 0) {
      const folderId = selectedKeys[0] as number;
      setCurrentFolderId(folderId);
      setSelectedFileIds([]);
      setSearchKeyword('');

      // Build breadcrumb path
      const path: { id: number; name: string }[] = [];
      let current = folders.find(f => f.folder_id === folderId);
      while (current) {
        path.unshift({ id: current.folder_id, name: current.name });
        current = current.parent_id ? folders.find(f => f.folder_id === current!.parent_id) : undefined;
      }
      setBreadcrumb(path);
    }
  };

  const handleImportFiles = async () => {
    try {
      const selected = await open({
        multiple: true,
        directory: false,
      });
      if (selected) {
        const paths = Array.isArray(selected) ? selected : [selected];
        const result = await invoke<ImportResult>('import_files', {
          filePaths: paths,
          folderId: currentFolderId,
        });
        if (result.fail_count > 0) {
          message.warning(`导入完成：成功 ${result.success_count} 个，失败 ${result.fail_count} 个`);
        } else {
          message.success(`成功导入 ${result.success_count} 个文件`);
        }
        loadFiles(currentFolderId);
        loadFolders();
      }
    } catch (e) {
      message.error('导入失败: ' + e);
    }
  };

  const handleImportFolders = async () => {
    try {
      const selected = await open({
        multiple: true,
        directory: true,
      });
      if (selected) {
        const paths = Array.isArray(selected) ? selected : [selected];
        console.log('Importing folders:', paths);
        const result = await invoke<ImportResult>('import_files', {
          filePaths: paths,
          folderId: currentFolderId,
        });
        console.log('Import result:', result);
        if (result.fail_count > 0) {
          message.warning(`导入完成：成功 ${result.success_count} 个，失败 ${result.fail_count} 个`);
          if (result.errors.length > 0) {
            console.error('Import errors:', result.errors);
          }
        } else {
          message.success(`成功导入 ${result.success_count} 个文件，请查看左侧目录树`);
        }
        // Refresh both files and folders
        await loadFolders();
        await loadFiles(currentFolderId);
      }
    } catch (e) {
      console.error('Import failed:', e);
      message.error('导入失败: ' + e);
    }
  };

  const handleExport = async () => {
    if (selectedFileIds.length === 0) {
      message.warning('请先选择要导出的文件');
      return;
    }
    try {
      const targetDir = await open({
        multiple: false,
        directory: true,
      });
      if (targetDir) {
        const dir = Array.isArray(targetDir) ? targetDir[0] : targetDir;
        await invoke<string[]>('export_files', {
          fileIds: selectedFileIds,
          targetDir: dir,
        });
        message.success(`成功导出 ${selectedFileIds.length} 个文件`);
      }
    } catch (e) {
      message.error('导出失败: ' + e);
    }
  };

  const handleDelete = async (ids?: number[]) => {
    const targetIds = ids || selectedFileIds;
    if (targetIds.length === 0) return;
    try {
      for (const id of targetIds) {
        await invoke('delete_file', { fileId: id });
      }
      message.success(`已删除 ${targetIds.length} 个文件`);
      setSelectedFileIds([]);
      loadFiles(currentFolderId);
    } catch (e) {
      message.error('删除失败: ' + e);
    }
  };

  const handleOpenFile = async (fileId: number) => {
    try {
      await invoke('open_file', { fileId });
    } catch (e) {
      message.error('打开文件失败: ' + e);
    }
  };

  const handleSearch = async () => {
    if (!searchKeyword.trim()) {
      loadFiles(currentFolderId);
      return;
    }
    setLoading(true);
    try {
      const result = await invoke<FileInfo[]>('search_files', { keyword: searchKeyword });
      setFiles(result);
    } catch (e) {
      console.error('Search failed:', e);
    } finally {
      setLoading(false);
    }
  };

  const handleCreateFolder = async () => {
    if (!newFolderName.trim()) return;
    try {
      await invoke('create_folder', {
        name: newFolderName,
        parentId: currentFolderId,
      });
      setNewFolderModalOpen(false);
      setNewFolderName('');
      loadFolders();
      message.success('文件夹创建成功');
    } catch (e) {
      message.error('创建文件夹失败: ' + e);
    }
  };

  const handleRename = async () => {
    if (!renameValue.trim() || renameFileId === null) return;
    try {
      await invoke('rename_file', { fileId: renameFileId, newName: renameValue });
      setRenameModalOpen(false);
      setRenameValue('');
      setRenameFileId(null);
      loadFiles(currentFolderId);
      message.success('重命名成功');
    } catch (e) {
      message.error('重命名失败: ' + e);
    }
  };

  const getContextMenuItems = (file: FileInfo): MenuProps['items'] => [
    {
      key: 'open',
      label: '打开',
      onClick: () => handleOpenFile(file.file_id),
    },
    { type: 'divider' },
    {
      key: 'rename',
      label: '重命名',
      icon: <EditOutlined />,
      onClick: () => {
        setRenameFileId(file.file_id);
        setRenameValue(file.name);
        setRenameModalOpen(true);
      },
    },
    {
      key: 'delete',
      label: '删除',
      icon: <DeleteOutlined />,
      danger: true,
      onClick: () => {
        Modal.confirm({
          title: '确认删除',
          content: `确定要删除 "${file.name}" 吗？`,
          onOk: () => handleDelete([file.file_id]),
        });
      },
    },
    { type: 'divider' },
    {
      key: 'export',
      label: '导出到...',
      icon: <ExportOutlined />,
      onClick: () => {
        setSelectedFileIds([file.file_id]);
        handleExport();
      },
    },
  ];

  const columns = [
    {
      title: '文件名',
      dataIndex: 'name',
      key: 'name',
      render: (name: string, record: FileInfo) => (
        <span>
          {getFileIcon(record.category)} {name}
        </span>
      ),
    },
    {
      title: '大小',
      dataIndex: 'size_bytes',
      key: 'size',
      width: 100,
      render: (size: number) => formatFileSize(size),
    },
    {
      title: '类型',
      dataIndex: 'category',
      key: 'category',
      width: 80,
      render: (cat: string) => {
        switch (cat) {
          case 'image': return '图片';
          case 'video_short': return '短视频';
          case 'video_long': return '长视频';
          case 'doc': return '文档';
          default: return cat || '-';
        }
      },
    },
    {
      title: '修改时间',
      dataIndex: 'updated_at',
      key: 'updated_at',
      width: 180,
      render: (t: number) => formatTime(t),
    },
  ];

  return (
    <Layout style={{ height: '100vh' }}>
      {/* Toolbar */}
      <Layout.Header style={{ background: '#fff', padding: '0 16px', borderBottom: '1px solid #f0f0f0', display: 'flex', alignItems: 'center', justifyContent: 'space-between', height: 48 }}>
        <Space>
          <Tooltip title="导入文件">
            <Button icon={<ImportOutlined />} onClick={handleImportFiles}>导入文件</Button>
          </Tooltip>
          <Tooltip title="导入文件夹">
            <Button icon={<FolderAddOutlined />} onClick={handleImportFolders}>导入文件夹</Button>
          </Tooltip>
          <Tooltip title="导出选中">
            <Button icon={<ExportOutlined />} onClick={handleExport} disabled={selectedFileIds.length === 0}>导出</Button>
          </Tooltip>
          <Tooltip title="删除选中">
            <Button icon={<DeleteOutlined />} onClick={() => handleDelete()} danger disabled={selectedFileIds.length === 0}>删除</Button>
          </Tooltip>
          <Tooltip title="新建文件夹">
            <Button icon={<FolderAddOutlined />} onClick={() => setNewFolderModalOpen(true)}>新建文件夹</Button>
          </Tooltip>
          <Tooltip title="刷新">
            <Button icon={<ReloadOutlined />} onClick={() => loadFiles(currentFolderId)} />
          </Tooltip>
        </Space>
        <Space>
          <Input.Search
            placeholder="搜索文件名..."
            value={searchKeyword}
            onChange={e => setSearchKeyword(e.target.value)}
            onSearch={handleSearch}
            style={{ width: 250 }}
            allowClear
          />
          <Button
            icon={viewMode === 'list' ? <UnorderedListOutlined /> : <AppstoreOutlined />}
            onClick={() => setViewMode(viewMode === 'list' ? 'grid' : 'list')}
          />
        </Space>
      </Layout.Header>

      <Layout style={{ flex: 1, overflow: 'hidden' }}>
        {/* Sidebar - Directory Tree */}
        <Layout.Sider width={240} style={{ background: '#fff', borderRight: '1px solid #f0f0f0', overflow: 'auto' }}>
          <div style={{ padding: '8px 12px', fontWeight: 'bold', borderBottom: '1px solid #f0f0f0' }}>
            <HomeOutlined /> 目录
          </div>
          <Tree
            showIcon
            defaultExpandAll
            treeData={buildTreeData(folders)}
            selectedKeys={[currentFolderId]}
            onSelect={handleFolderSelect}
            style={{ padding: '8px' }}
          />
        </Layout.Sider>

        {/* Main Content */}
        <Layout.Content style={{ overflow: 'auto', background: '#fff' }}>
          {/* Breadcrumb */}
          <div style={{ padding: '8px 16px', borderBottom: '1px solid #f0f0f0' }}>
            <Breadcrumb>
              {breadcrumb.map((item, idx) => (
                <Breadcrumb.Item
                  key={item.id}
                  onClick={() => {
                    setCurrentFolderId(item.id);
                    setBreadcrumb(breadcrumb.slice(0, idx + 1));
                  }}
                  style={{ cursor: 'pointer' }}
                >
                  {item.name}
                </Breadcrumb.Item>
              ))}
            </Breadcrumb>
          </div>

          {/* File List / Grid */}
          {viewMode === 'list' ? (
            <Table
              dataSource={files}
              columns={columns}
              rowKey="file_id"
              loading={loading}
              pagination={false}
              size="small"
              rowSelection={{
                selectedRowKeys: selectedFileIds,
                onChange: (keys) => setSelectedFileIds(keys as number[]),
              }}
              onRow={(record) => ({
                onDoubleClick: () => handleOpenFile(record.file_id),
                onContextMenu: (e) => {
                  e.preventDefault();
                },
              })}
              scroll={{ y: 'calc(100vh - 180px)' }}
            />
          ) : (
            <div style={{ padding: 16, display: 'flex', flexWrap: 'wrap', gap: 12, overflow: 'auto' }}>
              {files.map(file => (
                <Dropdown
                  key={file.file_id}
                  menu={{ items: getContextMenuItems(file) }}
                  trigger={['contextMenu']}
                >
                  <div
                    style={{
                      width: 120,
                      padding: 8,
                      textAlign: 'center',
                      borderRadius: 4,
                      border: selectedFileIds.includes(file.file_id) ? '2px solid #1890ff' : '1px solid #f0f0f0',
                      cursor: 'pointer',
                      background: selectedFileIds.includes(file.file_id) ? '#e6f7ff' : '#fff',
                    }}
                    onDoubleClick={() => handleOpenFile(file.file_id)}
                    onClick={(e) => {
                      if (e.ctrlKey) {
                        setSelectedFileIds(prev =>
                          prev.includes(file.file_id)
                            ? prev.filter(id => id !== file.file_id)
                            : [...prev, file.file_id]
                        );
                      } else {
                        setSelectedFileIds([file.file_id]);
                      }
                    }}
                  >
                    <div style={{ fontSize: 40, marginBottom: 4 }}>
                      {getFileIcon(file.category)}
                    </div>
                    <div style={{
                      fontSize: 12,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}>
                      {file.name}
                    </div>
                    <div style={{ fontSize: 11, color: '#999' }}>
                      {formatFileSize(file.size_bytes)}
                    </div>
                  </div>
                </Dropdown>
              ))}
            </div>
          )}
        </Layout.Content>
      </Layout>

      {/* Status Bar */}
      <Layout.Footer style={{ height: 28, padding: '0 16px', background: '#fafafa', borderTop: '1px solid #f0f0f0', display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 12, color: '#666', lineHeight: '28px' }}>
        <span>{files.length} 个文件</span>
        <span>
          {selectedFileIds.length > 0 && `已选择 ${selectedFileIds.length} 个 | `}
          总大小: {formatFileSize(files.reduce((sum, f) => sum + f.size_bytes, 0))}
        </span>
      </Layout.Footer>

      {/* Rename Modal */}
      <Modal
        title="重命名"
        open={renameModalOpen}
        onOk={handleRename}
        onCancel={() => setRenameModalOpen(false)}
      >
        <Input
          value={renameValue}
          onChange={e => setRenameValue(e.target.value)}
          onPressEnter={handleRename}
        />
      </Modal>

      {/* New Folder Modal */}
      <Modal
        title="新建文件夹"
        open={newFolderModalOpen}
        onOk={handleCreateFolder}
        onCancel={() => setNewFolderModalOpen(false)}
      >
        <Input
          placeholder="文件夹名称"
          value={newFolderName}
          onChange={e => setNewFolderName(e.target.value)}
          onPressEnter={handleCreateFolder}
        />
      </Modal>
    </Layout>
  );
}

export default App;
