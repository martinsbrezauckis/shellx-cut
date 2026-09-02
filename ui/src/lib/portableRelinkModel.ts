/** Durable B5 evidence reconstructed from the current grouped relink operation.
 * It is transient `project.state` data: the operation log remains the source
 * of truth, while Projects passes this exact value into B6 plan/create. */
export interface PortableB5RelinkReceipt {
  schema: 'shellx-cut/media-relink-receipt/1'
  immutable: true
  project_identity: {
    schema: 'shellx-cut/project-identity/1'
    origin_path_sha256: string
    project_name: string
  }
  pre_revision: string
  post_revision: string
  plan_hash: string
  grouped_op_id: string
  assets: Array<{
    asset: string
    expected_hash: string
    chosen: { path: string; sha256: string }
    disposition: 'relinked'
  }>
  scope: {
    library_transaction: false
    import_or_proxy_job: false
    undo: 'not_promised'
  }
}
