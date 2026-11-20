import { createRoot } from 'react-dom/client'
import App from '../../src/App'
import '../../src/theme.css'

const root = createRoot(document.getElementById('root')!)
root.render(<App />)
;(window as Window & { __cutUnmount?: () => void }).__cutUnmount = () => root.unmount()
